use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::complete_multipart_upload::CompleteMultipartUploadError;
use aws_sdk_s3::operation::create_multipart_upload::{
    CreateMultipartUploadError, CreateMultipartUploadOutput,
};
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use aws_sdk_s3::Client;
use bytesize::{GIB, MIB, TIB};
use conv::{prelude::*, FloatError, RangeError};
use derive_more::{AsRef, Display, Into};
use either::Either;
use futures::stream::TryStreamExt;
use std::error::Error as StdError;
use std::num::TryFromIntError;
use std::sync::Arc;
use thiserror::Error;
use tracing::instrument;
use typed_builder::TypedBuilder;

use super::S3Object;

/// Retrieves the size of the source object in the specified S3 bucket.
///
/// # Arguments
///
/// * `client` - A reference to the S3 client.
/// * `bucket` - The name of the S3 bucket.
/// * `key` - The key of the source object.
///
/// # Returns
///
/// The size of the source object in bytes.
#[instrument(skip(client))]
async fn get_source_size(
    client: &Client,
    bucket: &str,
    key: &str,
) -> Result<SourceSize, S3MultipartCopierError> {
    let head_object = client.head_object().bucket(bucket).key(key).send().await?;

    let length = head_object
        .content_length()
        .ok_or(S3MultipartCopierError::MissingContentLength)?;
    let source = SourceSize::try_from(length).map_err(S3MultipartCopierError::SourceSize)?;
    Ok(source)
}

/// The minimum allowed source size for an S3 object, set to 0 bytes.
///
/// This constant defines the minimum size an S3 object can be for operations that
/// require a size check. Setting it to 0 bytes allows for empty objects, which are
/// valid in S3 but might be restricted in some specific use cases.
const MIN_SOURCE_SIZE: i64 = 0;

/// The maximum allowed source size for an S3 object, set to 5 TiB.
///
/// This constant defines the maximum size an S3 object can be for operations that
/// require a size check. S3 objects can be up to 5 TiB (5 * 1024 * 1024 * 1024 * 1024 bytes) in size.
/// This limit ensures that objects are manageable and within S3's service constraints.
const MAX_SOURCE_SIZE: i64 = 5 * TIB as i64;

/// Errors that can occur when creating a `SourceSize`.
///
/// `SourceSizeError` is used to indicate that a given size is either too small or too large
/// to be used as an S3 object size.
///
/// # Variants
///
/// - `TooSmall(i64)`: Indicates that the source size is smaller than the minimum allowed size.
/// - `TooLarge(i64)`: Indicates that the source size is larger than the maximum allowed size.
#[derive(Debug, Error)]
pub enum SourceSizeError {
    #[error("S3 Object must be at least {MIN_SOURCE_SIZE} bytes. Object size was {0}")]
    TooSmall(i64),
    #[error("S3 Object must be at most {MAX_SOURCE_SIZE} bytes, Object size was {0}")]
    TooLarge(i64),
}

/// Represents a valid source size for an S3 object.
///
/// `SourceSize` ensures that the size of an S3 object is within the allowed range defined by S3.
/// The size must be at least 0 bytes and at most 5 TiB.
#[derive(Debug, Display, Into, AsRef, Clone, Eq, PartialEq)]
#[into(owned, ref, ref_mut)]
pub struct SourceSize(i64);

/// Attempts to create a `SourceSize` from an `i64` value.
///
/// The `TryFrom<i64>` implementation for `SourceSize` ensures that the given value is within
/// the allowed range for S3 object sizes. If the value is within the range, it returns `Ok(SourceSize)`.
/// Otherwise, it returns an appropriate `SourceSizeError`.
///
/// # Errors
///
/// - Returns `SourceSizeError::TooSmall` if the value is smaller than `MIN_SOURCE_SIZE`.
/// - Returns `SourceSizeError::TooLarge` if the value is larger than `MAX_SOURCE_SIZE`.
impl TryFrom<i64> for SourceSize {
    type Error = SourceSizeError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if value < MIN_SOURCE_SIZE {
            Err(SourceSizeError::TooSmall(value))
        } else if value > MAX_SOURCE_SIZE {
            Err(SourceSizeError::TooLarge(value))
        } else {
            Ok(SourceSize(value))
        }
    }
}

/// The minimum allowed part size for S3 multipart uploads, set to 5 MiB.
///
/// S3 enforces a minimum part size of 5 MiB (5 * 1024 * 1024 bytes) for multipart uploads.
/// Parts smaller than this size are not allowed, except for the last part of the upload.
/// Ensuring that each part meets this minimum size requirement helps optimize the upload process
/// and ensures compatibility with S3's multipart upload API.
const MIN_PART_SIZE: i64 = 5 * MIB as i64;

// The maximum allowed part size for S3 multipart uploads, set to 5 GiB.
///
/// S3 enforces a maximum part size of 5 GiB (5 * 1024 * 1024 * 1024 bytes) for multipart uploads.
/// Parts larger than this size are not allowed. This limitation helps prevent excessively large
/// parts from overwhelming the upload process and ensures that the upload is broken down into manageable
/// chunks. Adhering to this maximum size requirement is essential for successful multipart uploads.
const MAX_PART_SIZE: i64 = 5 * GIB as i64;

/// Represents a valid part size for S3 multipart uploads.
///
/// `PartSize` ensures that the size of each part used in multipart uploads is within
/// the allowed range defined by S3. The size must be at least 5 MB and at most 5 GB.
///
/// # Constants
///
/// - `MIN_PART_SIZE`: The minimum allowed part size (5 MB).
/// - `MAX_PART_SIZE`: The maximum allowed part size (5 GB).
#[derive(Debug, Into, AsRef, Clone)]
#[into(owned, ref, ref_mut)]
pub struct PartSize(i64);

/// The default size of a block for S3 multipart copy operations, set to 50 MiB.
///
/// # Why 50 MiB?
///
/// - **Balance**: Optimizes between throughput and the number of API calls.
/// - **S3 Limits**: Fits within S3's part size requirements (min 5 MiB, max 5 TB).
/// - **Parallelism**: Allows efficient parallel uploads, speeding up the copy process.
/// - **Error Handling**: Facilitates easier retries of failed parts without re-uploading the entire object.
///
/// This size ensures efficient, cost-effective, and reliable multipart copy operations.
///
/// # Note
///
/// While 50 MiB is a good default for many use cases, it might not be suitable for all operations.
/// Adjust the part size based on your specific requirements and constraints.
impl Default for PartSize {
    fn default() -> Self {
        const DEFAULT_COPY_PART_SIZE: i64 = 50 * MIB as i64;
        Self(DEFAULT_COPY_PART_SIZE)
    }
}

#[derive(Debug, Error)]
pub enum PartSizeError {
    #[error("part_size must be at least {MIN_PART_SIZE} bytes. part_size was {0}")]
    TooSmall(i64),
    #[error("part_size must be at most {MAX_PART_SIZE} bytes, part_size was {0}")]
    TooLarge(i64),
}

/// Attempts to create a `PartSize` from an `i64` value.
///
/// The `TryFrom<i64>` implementation for `PartSize` ensures that the given value is within
/// the allowed range for S3 multipart upload part sizes. If the value is within the range,
/// it returns `Ok(PartSize)`. Otherwise, it returns an appropriate `PartSizeError`.
///
/// # Errors
///
/// - Returns `PartSizeError::TooSmall` if the value is smaller than `MIN_PART_SIZE`.
/// - Returns `PartSizeError::TooLarge` if the value is larger than `MAX_PART_SIZE`.
impl TryFrom<i64> for PartSize {
    type Error = PartSizeError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if value < MIN_PART_SIZE {
            Err(PartSizeError::TooSmall(value))
        } else if value > MAX_PART_SIZE {
            Err(PartSizeError::TooLarge(value))
        } else {
            Ok(PartSize(value))
        }
    }
}

/// Errors that can occur when creating a `ByteRange`.
///
/// This enum captures the possible validation errors for a `ByteRange`.
#[derive(Debug, Error)]
pub enum ByteRangeError {
    #[error("The start byte must be less than or equal to the end byte \n start: {0}, end: {1}")]
    InvalidRange(i64, i64),
    #[error("The start byte must be non-negative: \n start {0}")]
    NegativeStart(i64),
}

/// A struct representing a byte range.
///
/// `ByteRange` is used to define a range of bytes, typically for operations such as
/// downloading a specific portion of an object from S3. It includes validation to ensure
/// the byte range is valid, with the start byte less than or equal to the end byte and
/// both bytes being non-negative.
#[derive(Debug, Clone, Copy)]
pub struct ByteRange(i64, i64);

impl TryFrom<(i64, i64)> for ByteRange {
    type Error = ByteRangeError;

    fn try_from(value: (i64, i64)) -> Result<Self, Self::Error> {
        let (start, end) = value;

        if start < 0 {
            Err(ByteRangeError::NegativeStart(start))
        } else if start > end {
            Err(ByteRangeError::InvalidRange(start, end))
        } else {
            Ok(ByteRange(start, end))
        }
    }
}

impl ByteRange {
    /// Generates a byte range string for S3 operations.
    ///
    /// # Returns
    ///
    /// A string representing the byte range.
    ///
    /// # Examples
    ///
    /// ```
    /// let range = ByteRange::try_from((0, 499)).unwrap();
    /// assert_eq!(range.to_string(), "bytes=0-499");
    ///
    /// let range = ByteRange::try_from((500, 999)).unwrap();
    /// assert_eq!(range.as_string(), "bytes=500-999");
    /// ```
    pub fn as_string(&self) -> String {
        let ByteRange(start, end) = self;
        format!("bytes={}-{}", start, end)
    }
}

/// Custom error types for S3 multipart copy operations.
#[derive(Debug, Error)]
pub enum S3MultipartCopierError {
    #[error("Missing multipart upload id")]
    MissingUploadId,
    #[error("Missing copy part result")]
    MissingCopyPartResult,
    #[error("Missing content length")]
    MissingContentLength,
    #[error(transparent)]
    RangeError(#[from] RangeError<i64>),
    #[error(transparent)]
    FloatError(#[from] FloatError<f64>),
    #[error(transparent)]
    TryFromIntError(#[from] TryFromIntError),
    #[error(transparent)]
    SourceSize(#[from] SourceSizeError),
    #[error("PartSize larger than SourceSize \n Atomic copy should be use. part_size : {part_size}, source_size : {source_size}")]
    PartSizeGreaterThanSource { part_size: i64, source_size: i64 },
    #[error("Can not perform multipart copy with source size 0")]
    MultipartCopySourceSizeZero,
    #[error(transparent)]
    ByteRangeError(#[from] ByteRangeError),
    #[error(transparent)]
    S3Error(Box<dyn StdError + Send + Sync>),
}

impl<E: StdError + Send + Sync + 'static> From<SdkError<E>> for S3MultipartCopierError {
    fn from(value: SdkError<E>) -> Self {
        Self::S3Error(Box::new(value))
    }
}

/// A struct representing the parameters required for copying a part of an S3 object.
#[derive(Debug, TypedBuilder)]
struct CopyUploadPart<'a> {
    src: &'a S3Object,
    dst: &'a S3Object,
    upload_id: &'a str,
    part_number: i32,
    byte_range: ByteRange,
}

/// A struct to handle S3 multipart copy operations.
///
/// `S3MultipartCopier` facilitates copying large objects in S3 by breaking them into
/// smaller parts and uploading them in parallel. This is particularly useful for objects
/// larger than 5 GB, as S3's single-part copy operation is limited to this size. If the
/// source file is smaller than the part size, an atomic copy will be used instead, which
/// involves calling the S3 copy API to perform the copy operation in a single request.
///
/// # Fields
///
/// - `client`: An `Arc`-wrapped S3 `Client` used to perform the copy operations.
/// - `part_size`: The size of each part in bytes. Defaults to `DEFAULT_COPY_PART_SIZE` (50 MiB).
/// - `max_concurrent_uploads`: The maximum number of parts to upload concurrently.
/// - `source`: The `S3Object` representing the source object to copy.
/// - `destination`: The `S3Object` representing the destination object.
///
/// # Example
///
/// ```rust
/// use aws_sdk_s3::Client;
/// use std::sync::Arc;
/// use aws_cobalt::s3::{S3MultipartCopier, S3Object, PartSize};
///
/// let client = Arc::new(Client::new(&shared_config));
/// let source = S3Object::new("source-bucket", "source-key");
/// let destination = S3Object::new("destination-bucket", "destination-key");
///
/// let copier = S3MultipartCopier::builder()
///     .client(client)
///     .part_size(PartSize::try_from(50 * 1024 * 1024).unwrap()) // 50 MiB
///     .max_concurrent_uploads(4)
///     .source(source)
///     .destination(destination)
///     .build();
///
/// copier.send().await?;
/// ```
///
/// # Note
///
/// Ensure that the `part_size` is appropriate for your use case. While 50 MiB is a good default,
/// it might not be suitable for all operations. Adjust the part size based on your specific
/// requirements and constraints. Additionally, if the source file is smaller than the part size,
/// an atomic copy will be used instead of a multipart copy. An atomic copy involves calling the
/// S3 copy API to perform the copy operation in a single request.

#[derive(Debug, TypedBuilder)]
pub struct S3MultipartCopier {
    client: Arc<Client>,
    #[builder(default=PartSize::default())]
    part_size: PartSize,
    max_concurrent_uploads: usize,
    source: S3Object,
    destination: S3Object,
}

impl S3MultipartCopier {
    /// Initiates a multipart upload to the specified S3 bucket.
    ///
    /// # Arguments
    ///
    /// * `bucket` - The name of the destination S3 bucket.
    /// * `key` - The key of the destination object.
    ///
    /// # Returns
    ///
    /// The output of the multipart upload initiation.
    #[instrument(skip(self))]
    async fn initiate_multipart_upload(
        &self,
    ) -> Result<CreateMultipartUploadOutput, SdkError<CreateMultipartUploadError>> {
        self.client
            .create_multipart_upload()
            .bucket(&self.destination.bucket)
            .key(&self.destination.key)
            .send()
            .await
    }

    fn copy_source(object: &S3Object) -> String {
        format!("{}/{}", object.bucket, object.key)
    }

    /// Uploads a part of the source object to the destination as part of the multipart upload.
    ///
    /// # Arguments
    ///
    /// * `part` - The `CopyUploadPart` containing the parameters for the upload.
    ///
    /// # Returns
    ///
    /// The completed part containing the ETag and part number.
    #[instrument(skip(self))]
    async fn upload_part_copy(
        &self,
        part: CopyUploadPart<'_>,
    ) -> Result<CompletedPart, S3MultipartCopierError> {
        let copy_source = S3MultipartCopier::copy_source(part.src);

        let response = self
            .client
            .upload_part_copy()
            .bucket(&part.dst.bucket)
            .key(&part.dst.key)
            .part_number(part.part_number)
            .upload_id(part.upload_id)
            .copy_source(copy_source)
            .copy_source_range(part.byte_range.as_string())
            .send()
            .await?;

        Ok(CompletedPart::builder()
            .set_e_tag(
                response
                    .copy_part_result
                    .ok_or(S3MultipartCopierError::MissingCopyPartResult)?
                    .e_tag,
            )
            .part_number(part.part_number)
            .build())
    }

    /// Completes the multipart upload by combining all parts.
    ///
    /// # Arguments
    ///
    /// * `upload_id` - The upload ID of the multipart upload.
    /// * `parts` - A vector of completed parts.
    ///
    /// # Returns
    ///
    /// An empty result indicating success.
    #[instrument(skip(self))]
    async fn complete_multipart_upload(
        &self,
        upload_id: &str,
        mut parts: Vec<CompletedPart>,
    ) -> Result<(), SdkError<CompleteMultipartUploadError>> {
        parts.sort_by_key(|part| part.part_number);
        let completed_multipart_upload = CompletedMultipartUpload::builder()
            .set_parts(Some(parts))
            .build();

        self.client
            .complete_multipart_upload()
            .bucket(&self.destination.bucket)
            .key(&self.destination.key)
            .upload_id(upload_id)
            .multipart_upload(completed_multipart_upload)
            .send()
            .await?;

        Ok(())
    }

    /// Performs a multipart copy of a large object from the source bucket to the destination bucket.
    ///
    /// # Returns
    ///
    /// An empty result indicating success.
    #[instrument(skip(self))]
    pub async fn send(&self) -> Result<(), S3MultipartCopierError> {
        tracing::info!("Starting multipart copy");
        let source_size =
            get_source_size(&self.client, &self.source.bucket, &self.source.key).await?;

        tracing::info!(
            source_size = source_size.as_ref(),
            part_size = self.part_size.as_ref(),
        );

        if source_size.as_ref() <= self.part_size.as_ref() {
            tracing::info!("Source size is smaller than part size, using atomic copy");
            self.atomic_copy().await
        } else {
            tracing::info!("Source size is larger than part size, using multipart copy");
            self.multipart_copy(&source_size).await
        }
    }

    async fn atomic_copy(&self) -> Result<(), S3MultipartCopierError> {
        let copy_source = S3MultipartCopier::copy_source(&self.source);
        self.client
            .copy_object()
            .copy_source(copy_source)
            .bucket(&self.destination.bucket)
            .key(&self.destination.key)
            .send()
            .await?;
        Ok(())
    }

    async fn multipart_copy(&self, source_size: &SourceSize) -> Result<(), S3MultipartCopierError> {
        if self.part_size.as_ref() > source_size.as_ref() {
            return Err(S3MultipartCopierError::PartSizeGreaterThanSource {
                part_size: *self.part_size.as_ref(),
                source_size: *source_size.as_ref(),
            });
        }

        let create_multipart_upload = self.initiate_multipart_upload().await?;
        let upload_id = create_multipart_upload
            .upload_id()
            .ok_or(S3MultipartCopierError::MissingUploadId)?;

        let parts = futures::stream::iter(Self::byte_ranges(source_size, &self.part_size))
            .map_ok(|(part_number, byte_range)| {
                let source = &self.source;
                let destination = &self.destination;
                let upload_id = upload_id.to_string();

                async move {
                    tracing::info!(byte_range = ?byte_range);

                    let part = CopyUploadPart::builder()
                        .src(source)
                        .dst(destination)
                        .upload_id(&upload_id)
                        .part_number(i32::try_from(part_number)?)
                        .byte_range(byte_range)
                        .build();
                    tracing::debug!(part = ?part, "Copying");
                    self.upload_part_copy(part).await
                }
            })
            .try_buffer_unordered(self.max_concurrent_uploads);

        let completed_parts: Vec<CompletedPart> = parts.try_collect().await?;

        tracing::info!(upload_id = upload_id, "All parts completed");
        self.complete_multipart_upload(upload_id, completed_parts)
            .await?;

        tracing::info!("MultipartCopy completed");
        Ok(())
    }

    fn byte_ranges<'a>(
        source_size: &'a SourceSize,
        part_size: &'a PartSize,
    ) -> impl Iterator<Item = Result<(i64, ByteRange), S3MultipartCopierError>> + 'a {
        if *source_size.as_ref() == 0 {
            Either::Left(std::iter::once(Err(
                S3MultipartCopierError::MultipartCopySourceSizeZero,
            )))
        } else {
            let part_count = match S3MultipartCopier::part_count(source_size, part_size) {
                Ok(count) => count,
                Err(e) => return Either::Left(std::iter::once(Err(e))),
            };
            Either::Right((1..=part_count).map(move |part_number| {
                let part_size = *part_size.as_ref();
                let source_size = *source_size.as_ref();

                let byte_range_start = (part_number - 1) * part_size;
                let byte_range_end = std::cmp::min(part_number * part_size - 1, source_size - 1);

                let byte_range = ByteRange::try_from((byte_range_start, byte_range_end))?;
                Ok((part_number, byte_range))
            }))
        }
    }

    fn part_count(
        source_size: &SourceSize,
        part_size: &PartSize,
    ) -> Result<i64, S3MultipartCopierError> {
        let source_size = *source_size.as_ref();
        let part_size = *part_size.as_ref();

        if source_size == 0 {
            return Err(S3MultipartCopierError::MultipartCopySourceSizeZero);
        }

        Ok(((f64::value_from(source_size)? / f64::value_from(part_size)?).ceil()).approx()?)
    }
}

#[cfg(test)]
pub mod arbitrary {
    use derive_more::{AsRef, From, Into};
    use proptest::prelude::*;

    use super::{
        PartSize, SourceSize, MAX_PART_SIZE, MAX_SOURCE_SIZE, MIN_PART_SIZE, MIN_SOURCE_SIZE,
    };

    impl Arbitrary for PartSize {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (MIN_PART_SIZE..=MAX_PART_SIZE)
                .prop_map(|size| PartSize::try_from(size).unwrap())
                .boxed()
        }
    }

    impl Arbitrary for SourceSize {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (MIN_SOURCE_SIZE..=MAX_SOURCE_SIZE)
                .prop_map(|size| SourceSize::try_from(size).unwrap())
                .boxed()
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, AsRef, Into, From)]
    pub struct NonZeroSourceSize(SourceSize);

    // Arbitrary implementation for NonZeroSourceSize
    impl Arbitrary for NonZeroSourceSize {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (1..=MAX_SOURCE_SIZE)
                .prop_map(|size| SourceSize::try_from(size).unwrap())
                .prop_map(NonZeroSourceSize)
                .boxed()
        }
    }
}

#[cfg(test)]
mod tests {

    use self::arbitrary::NonZeroSourceSize;

    use super::*;
    use crate::localstack;
    #[allow(deprecated)]
    use crate::s3::get_client;
    use crate::s3::test::*;
    use crate::s3::{AsyncMultipartUpload, S3Object};
    use ::function_name::named;
    use anyhow::Result;
    use aws_config;
    use aws_sdk_s3::Client;
    use bytesize::MIB;
    use futures::prelude::*;
    use proptest::{prop_assert, prop_assert_eq};
    use rand::Rng;
    use thiserror::Error;

    use test_strategy::proptest;

    //Wrapper to allow anyhow to be used a std::error::Error
    #[derive(Debug, Error)]
    #[error(transparent)]
    pub struct CustomError(#[from] anyhow::Error);

    // *** Integration tests *** //
    //Integration tests should be in src/tests but there is tight coupling with
    //localstack which makes it hard to migrate away from this structure.
    async fn localstack_test_client() -> Client {
        localstack::test_utils::wait_for_localstack().await;
        let shared_config = aws_config::load_from_env().await;
        #[allow(deprecated)]
        get_client(&shared_config).unwrap()
    }

    #[proptest(async = "tokio", cases = 3)]
    #[named]
    async fn test_multipart_copy(#[strategy(0_usize..=50*MIB as usize)] upload_size: usize) {
        let client = Arc::new(localstack_test_client().await);
        let test_bucket = "test-multipart-bucket";
        let mut rng = seeded_rng(function_name!());
        let src_key = gen_random_file_name(&mut rng);

        let result = create_bucket(&client, test_bucket).await;
        prop_assert!(result.is_ok(), "Error: {result:?}");

        let src = S3Object::new(test_bucket, &src_key);
        let src_bytes = generate_random_bytes(upload_size, &mut rng);

        let part_size = MIN_PART_SIZE as usize;

        let mut writer = AsyncMultipartUpload::new(&client, &src, part_size, None)
            .await
            .map_err(CustomError)?;
        writer.write_all(&src_bytes).await?;
        writer.close().await?;

        //prop_assert!(put_result.is_ok(), "Result : {put_result:?}");

        let dst_key = gen_random_file_name(&mut rng);

        let dest = S3Object::new(test_bucket, dst_key);
        let copyier = S3MultipartCopier::builder()
            .client(client.clone())
            .source(src)
            .destination(dest.clone())
            .part_size((5 * MIB as i64).try_into()?)
            .max_concurrent_uploads(100)
            .build();
        copyier.send().await?;

        let copied_bytes = fetch_bytes(&client, &dest).await.map_err(CustomError)?;
        prop_assert_eq!(src_bytes, copied_bytes);
    }

    #[tokio::test]
    #[named]
    async fn test_zero_size_multipart_copy() -> Result<()> {
        let client = Arc::new(localstack_test_client().await);
        let test_bucket = "test-multipart-bucket";
        let mut rng = seeded_rng(function_name!());
        let src_key = gen_random_file_name(&mut rng);

        create_bucket(&client, test_bucket).await?;

        let src = S3Object::new(test_bucket, &src_key);
        client
            .put_object()
            .bucket(test_bucket)
            .key(src_key)
            .body(Vec::default().into())
            .send()
            .await?;

        let dst_key = gen_random_file_name(&mut rng);

        let dest = S3Object::new(test_bucket, dst_key);
        let copyier = S3MultipartCopier::builder()
            .client(client.clone())
            .source(src)
            .destination(dest.clone())
            .part_size((5 * MIB as i64).try_into()?)
            .max_concurrent_uploads(2)
            .build();
        copyier.send().await?;
        let copied_bytes = fetch_bytes(&client, &dest).await?;
        assert_eq!(copied_bytes.len(), 0);

        Ok(())
    }

    fn generate_random_bytes(length: usize, rng: &mut impl Rng) -> Vec<u8> {
        (0..length).map(|_| rng.gen()).collect()
    }

    #[proptest]
    fn test_part_count_valid(non_zero_source: NonZeroSourceSize, part_size: PartSize) {
        let source_size = non_zero_source.into();
        let count = S3MultipartCopier::part_count(&source_size, &part_size)?;

        // Assert that the result is Ok and the part count is positive
        prop_assert!(count >= 0, "Expected count greater than 0");
        // Validate the part count is correct
        let expected_count = ((source_size.0 as f64) / (part_size.0 as f64)).ceil() as i64;
        prop_assert_eq!(count, expected_count);
    }

    #[proptest]
    fn test_part_count_small_source(part_size: PartSize, #[strategy(1_i64..=10)] source: i64) {
        let source_size = SourceSize(source);

        let count = S3MultipartCopier::part_count(&source_size, &part_size)?;
        prop_assert_eq!(count, 1, "Expected a part count of 1");
    }

    #[proptest]
    fn test_part_count_large_source(part_size: PartSize) {
        let source_size = SourceSize(MAX_SOURCE_SIZE); // Very large source size

        let count = S3MultipartCopier::part_count(&source_size, &part_size)?;

        // Assert that the result is Ok and the part count is positive
        prop_assert!(count >= 0, "Expected count greater than 0");

        let expected_count = ((source_size.0 as f64) / (part_size.0 as f64)).ceil() as i64;
        prop_assert_eq!(count, expected_count);
    }

    #[proptest]
    fn test_part_count_error_on_zero_source_size(part_size: PartSize) {
        let source_size = SourceSize(0); // Zero source size

        let result = S3MultipartCopier::part_count(&source_size, &part_size);

        // Assert that the result is an error and the error is MultipartCopySourceSizeZero
        prop_assert!(result.is_err());

        prop_assert!(matches!(
            result.unwrap_err(),
            S3MultipartCopierError::MultipartCopySourceSizeZero
        ));
    }

    #[proptest]
    fn test_source_size_within_limits(#[strategy(MIN_SOURCE_SIZE..=MAX_SOURCE_SIZE)] value: i64) {
        let source_size = SourceSize::try_from(value)?;
        prop_assert_eq!(source_size.as_ref(), &value);
    }

    #[proptest]
    fn test_source_size_too_small(#[strategy(i64::MIN..MIN_SOURCE_SIZE)] value: i64) {
        let source_size = SourceSize::try_from(value);
        prop_assert!(source_size.is_err());
        prop_assert!(
            matches!(source_size.unwrap_err(), SourceSizeError::TooSmall(v) if v == value)
        );
    }

    #[proptest]
    fn test_source_size_too_large(#[strategy((MAX_SOURCE_SIZE + 1)..=i64::MAX)] value: i64) {
        let source_size = SourceSize::try_from(value);
        prop_assert!(source_size.is_err());
        prop_assert!(
            matches!(source_size.unwrap_err(), SourceSizeError::TooLarge(v) if v == value)
        );
    }

    // PartSize
    #[proptest]
    fn test_part_size_within_limits(#[strategy(MIN_PART_SIZE..=MAX_PART_SIZE)] value: i64) {
        let part_size = PartSize::try_from(value)?;
        prop_assert_eq!(part_size.as_ref(), &value);
    }

    #[proptest]
    fn test_part_size_too_small(#[strategy(i64::MIN..MIN_PART_SIZE)] value: i64) {
        let part_size = PartSize::try_from(value);
        prop_assert!(part_size.is_err());
        prop_assert!(matches!(part_size.unwrap_err(), PartSizeError::TooSmall(v) if v == value));
    }

    #[proptest]
    fn test_part_size_too_large(#[strategy((MAX_PART_SIZE + 1)..=i64::MAX)] value: i64) {
        let part_size = PartSize::try_from(value);
        prop_assert!(part_size.is_err());
        prop_assert!(matches!(part_size.unwrap_err(), PartSizeError::TooLarge(v) if v == value));
    }

    //byte range
    #[proptest]
    fn valid_byte_range(#[strategy(0..i64::MAX)] start: i64, #[strategy(0..i64::MAX)] end: i64) {
        if start <= end {
            let range = ByteRange::try_from((start, end))?;
            prop_assert_eq!(range.0, start);
            prop_assert_eq!(range.1, end);
        } else {
            let range = ByteRange::try_from((start, end));
            prop_assert!(
                matches!(range, Err(ByteRangeError::InvalidRange(s, e)) if s == start && e == end)
            );
        }
    }

    #[proptest]
    fn invalid_negative_start_byte_range(
        #[strategy(i64::MIN..0)] start: i64,
        #[strategy(0..i64::MAX)] end: i64,
    ) {
        let range = ByteRange::try_from((start, end));
        prop_assert!(matches!(range, Err(ByteRangeError::NegativeStart(s)) if s == start));
    }

    #[proptest]
    fn invalid_byte_range_start_greater_than_end(
        #[strategy(0..i64::MAX)] start: i64,
        #[strategy(0..i64::MAX)] end: i64,
    ) {
        if start > end {
            let range = ByteRange::try_from((start, end));
            prop_assert!(
                matches!(range, Err(ByteRangeError::InvalidRange(s, e)) if s == start && e == end)
            );
        }
    }

    //bytes_ranges
    #[proptest]
    fn test_byte_ranges_valid(part_size: PartSize, non_zero_source: NonZeroSourceSize) {
        let source_size = non_zero_source.into();

        let result: Vec<_> = S3MultipartCopier::byte_ranges(&source_size, &part_size).collect();

        for (i, item) in result.iter().enumerate() {
            let (part_number, ByteRange(start, end)) = item.as_ref()?;
            prop_assert_eq!(*part_number as usize, i + 1);
            prop_assert!(start >= &0);
            prop_assert!(end >= start);
            prop_assert!(end < source_size.as_ref());
        }
        let mut expected_start = 0;
        for item in &result {
            let ByteRange(start, end) = item.as_ref()?.1;
            prop_assert_eq!(start, expected_start);
            expected_start = end + 1;
        }
        prop_assert_eq!(expected_start, *source_size.as_ref());
    }

    #[proptest]
    fn test_byte_ranges_zero_source_size(part_size: PartSize) {
        let source_size = SourceSize(0);

        let result: Vec<_> = S3MultipartCopier::byte_ranges(&source_size, &part_size).collect();

        prop_assert!(result.len() == 1);
        let err = result[0].as_ref().err().unwrap();
        prop_assert!(matches!(
            err,
            S3MultipartCopierError::MultipartCopySourceSizeZero
        ));
    }

    #[proptest]
    fn test_byte_ranges_large_source(
        part_size: PartSize,
        #[strategy(1_000_000_000_i64..10_000_000_000_i64)] source: i64,
    ) {
        let source_size = SourceSize(source);
        let result: Vec<_> = S3MultipartCopier::byte_ranges(&source_size, &part_size).collect();

        for (i, item) in result.iter().enumerate() {
            prop_assert!(item.is_ok(), "Error {:?}", item);
            let (part_number, ByteRange(start, end)) = item.as_ref()?;
            prop_assert_eq!(*part_number as usize, i + 1);
            prop_assert!(start >= &0);
            prop_assert!(end >= start);
            prop_assert!(end < source_size.as_ref());
        }
        let mut expected_start = 0;
        for item in &result {
            let ByteRange(start, end) = item.as_ref()?.1;
            prop_assert_eq!(start, expected_start);
            expected_start = end + 1;
        }
        prop_assert_eq!(expected_start, *source_size.as_ref());
    }
}
