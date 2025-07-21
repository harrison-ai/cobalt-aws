//! Provides ways of interacting with objects in S3.

// Standard library imports
use std::mem;
use std::pin::Pin;

// External crates
use anyhow::Context as _;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::{
    operation::{
        complete_multipart_upload::{CompleteMultipartUploadError, CompleteMultipartUploadOutput},
        upload_part::{UploadPartError, UploadPartOutput},
    },
    types::{CompletedMultipartUpload, CompletedPart, ObjectCannedAcl, Part},
    Client,
};
use bytesize::{GIB, MIB};
use derive_more::Debug;
use futures::{
    future::BoxFuture,
    io::Error,
    task::{Context, Poll},
    AsyncWrite, Future, FutureExt, TryFutureExt,
};
use tracing::{event, instrument, Level};

// Internal project imports
use crate::s3::S3Object;
use crate::types::SdkError;

/// Convenience wrapper for boxed future
type MultipartUploadFuture<'a> =
    BoxFuture<'a, Result<(UploadPartOutput, i32), SdkError<UploadPartError>>>;
/// Convenience wrapper for boxed future
type CompleteMultipartUploadFuture<'a> =
    BoxFuture<'a, Result<CompleteMultipartUploadOutput, SdkError<CompleteMultipartUploadError>>>;

/// Holds state for the [AsyncMultipartUpload]
#[derive(Debug)]
enum AsyncMultipartUploadState<'a> {
    /// Use this in mem::swap to avoid split borrows
    None,
    ///Bytes are being written
    Writing {
        /// Multipart Uploads that are running
        #[debug(skip)]
        uploads: Vec<MultipartUploadFuture<'a>>,
        /// Bytes waiting to be written.
        buffer: Vec<u8>,
        /// The next part number to be used.
        part_number: i32,
        /// The completed parts
        completed_parts: Vec<CompletedPart>,
    },
    /// close() has been called and parts are still uploading.
    CompletingParts {
        #[debug(skip)]
        uploads: Vec<MultipartUploadFuture<'a>>,
        completed_parts: Vec<CompletedPart>,
    },
    /// All parts have been uploaded and the CompleteMultipart is returning.
    Completing(#[debug(skip)] CompleteMultipartUploadFuture<'a>),
    // We have completed writing to S3.
    Closed,
}

/// A implementation of [AsyncWrite] for S3 objects using multipart uploads.
/// By using multipart uploads constant memory usage can be achieved.
///
/// ## Note
/// On failure the multipart upload is not aborted. It is up to the
/// caller to call the S3 `abortMultipartUpload` API when required.
///
/// ```no_run
/// use cobalt_aws::s3::{AsyncMultipartUpload, Client, S3Object};
/// use cobalt_aws::config::load_from_env;
/// use futures::AsyncWriteExt;
///
/// # tokio_test::block_on(async {
/// let shared_config = load_from_env().await.unwrap();
/// let client = Client::new(&shared_config);
/// let dst = S3Object::new("my-bucket", "my-key");
/// let part_size = 5 * 1_048_576;
/// let mut writer = AsyncMultipartUpload::new(&client, &dst, part_size, None).await.unwrap();
/// let buffer_len = 6 * 1_048_576;
/// // Each part is uploaded as it's available
/// writer.write_all(&vec![0; buffer_len]).await.unwrap();
///
/// // The pending parts are uploaded and the multipart upload is completed
/// // on close.
/// writer.close().await.unwrap();
/// # })
/// ```

#[derive(Debug)]
pub struct AsyncMultipartUpload<'a> {
    client: &'a Client,
    dst: S3Object,
    upload_id: String,
    part_size: usize,
    max_uploading_parts: usize,
    state: AsyncMultipartUploadState<'a>,
}

/// Minimum size of a multipart upload.
const MIN_PART_SIZE: usize = 5_usize * MIB as usize; // 5 Mib
/// Maximum size of a multipart upload.
const MAX_PART_SIZE: usize = 5_usize * GIB as usize; // 5 Gib

/// Default number of part which can be uploaded
/// concurrently.
const DEFAULT_MAX_UPLOADING_PARTS: usize = 100;

impl<'a> AsyncMultipartUpload<'a> {
    /// Create a new [AsyncMultipartUpload].
    ///
    /// * `client`              - S3 client to use.
    /// * `dst`                 - The [S3Object] to write the object into.
    /// * `part_size`           - How large, in bytes, each part should be. Must be
    /// larger than 5MIB and smaller that 5GIB.
    /// * `max_uploading_parts` - How many parts to upload concurrently,
    /// Must be larger than 0 (defaults to 100).
    #[instrument(skip(client))]
    pub async fn new(
        client: &'a Client,
        dst: &S3Object,
        part_size: usize,
        max_uploading_parts: Option<usize>,
    ) -> anyhow::Result<AsyncMultipartUpload<'a>> {
        event!(Level::DEBUG, "New AsyncMultipartUpload");
        if part_size < MIN_PART_SIZE {
            anyhow::bail!("part_size was {part_size}, can not be less than {MIN_PART_SIZE}")
        }
        if part_size > MAX_PART_SIZE {
            anyhow::bail!("part_size was {part_size}, can not be more than {MAX_PART_SIZE}")
        }

        //Check that user did not send in invalid parameter
        if let Some(0) = max_uploading_parts {
            anyhow::bail!("Max uploading parts must not be 0")
        }

        let result = client
            .create_multipart_upload()
            .bucket(&dst.bucket)
            .key(&dst.key)
            .acl(ObjectCannedAcl::BucketOwnerFullControl)
            .send()
            .await?;

        let upload_id = result.upload_id().context("Expected Upload Id")?;

        Ok(AsyncMultipartUpload {
            client,
            dst: dst.clone(),
            upload_id: upload_id.into(),
            part_size,
            max_uploading_parts: max_uploading_parts.unwrap_or(DEFAULT_MAX_UPLOADING_PARTS),
            state: AsyncMultipartUploadState::Writing {
                uploads: vec![],
                buffer: Vec::with_capacity(part_size),
                part_number: 1,
                completed_parts: vec![],
            },
        })
    }

    /// Import an existing [AsyncMultipartUpload] to continue uploading.
    ///
    /// * `client`              - S3 client to use.
    /// * `upload_id`           - The multipart upload ID to resume.
    /// * `dst`                 - The [S3Object] to write the object into.
    /// * `part_size`           - How large, in bytes, each part should be. Must be
    /// larger than 5MIB and smaller that 5GIB.
    /// * `max_uploading_parts` - How many parts to upload concurrently,
    /// Must be larger than 0 (defaults to 100).
    #[instrument(skip(client))]
    pub async fn from(
        client: &'a Client,
        upload_id: String,
        dst: &S3Object,
        part_size: usize,
        max_uploading_parts: Option<usize>,
    ) -> anyhow::Result<AsyncMultipartUpload<'a>> {
        event!(Level::DEBUG, "New AsyncMultipartUpload");
        if part_size < MIN_PART_SIZE {
            anyhow::bail!("part_size was {part_size}, can not be less than {MIN_PART_SIZE}")
        }
        if part_size > MAX_PART_SIZE {
            anyhow::bail!("part_size was {part_size}, can not be more than {MAX_PART_SIZE}")
        }

        //Check that user did not send in invalid parameter
        if let Some(0) = max_uploading_parts {
            anyhow::bail!("Max uploading parts must not be 0")
        }

        let mut parts: Vec<Part> = vec![];

        let mut list_parts_result = client
            .list_parts()
            .bucket(&dst.bucket)
            .key(&dst.key)
            .upload_id(&upload_id)
            .into_paginator()
            .send();

        // Use a paginator to collect all parts if there are more than 1000 parts in a multipart (the default and maximum part limit in list_parts())
        // You could use `.into_paginator().items().send()` here to get a flattened list of items, but this seems to not work
        // Any use of the items from that method results in a Future that never resolves, causing the program to hang indefinitely
        // So instead, we use this pagination method which is still alright!
        while let Some(Ok(page)) = list_parts_result.next().await {
            parts.append(&mut page.parts().to_vec());

            if !page.is_truncated().unwrap_or(false) {
                break;
            }
        }

        let completed_parts: Vec<CompletedPart> = parts
            .iter()
            .map(|part| {
                // Part has Option<&str> but CompletedPart has Option<String>
                // Convert any &str's to String's but retain them as Options still

                let e_tag = part.e_tag().map(|s| s.to_string());
                let checksum_crc32 = part.checksum_crc32().map(|s| s.to_string());
                let checksum_crc32_c = part.checksum_crc32_c().map(|s| s.to_string());
                let checksum_sha1 = part.checksum_sha1().map(|s| s.to_string());
                let checksum_sha256 = part.checksum_sha256().map(|s| s.to_string());

                Ok::<CompletedPart, anyhow::Error>(
                    CompletedPart::builder()
                        .set_e_tag(e_tag)
                        .set_checksum_crc32(checksum_crc32)
                        .set_checksum_crc32_c(checksum_crc32_c)
                        .set_checksum_sha1(checksum_sha1)
                        .set_checksum_sha256(checksum_sha256)
                        .part_number(part.part_number().context("Expected part number")?)
                        .build(),
                )
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let latest_part_number = parts
            .iter()
            .filter_map(|p| p.part_number())
            .max()
            .unwrap_or(0)
            + 1;

        Ok(AsyncMultipartUpload {
            client,
            dst: dst.clone(),
            upload_id: upload_id.clone(),
            part_size,
            max_uploading_parts: max_uploading_parts.unwrap_or(DEFAULT_MAX_UPLOADING_PARTS),
            state: AsyncMultipartUploadState::Writing {
                uploads: vec![],
                buffer: Vec::with_capacity(part_size),
                part_number: latest_part_number,
                completed_parts,
            },
        })
    }

    pub fn get_upload_id(&self) -> &str {
        self.upload_id.as_str()
    }

    #[instrument(skip(buffer))]
    fn upload_part<'b>(&self, buffer: Vec<u8>, part_number: i32) -> MultipartUploadFuture<'b> {
        event!(Level::DEBUG, "Uploading Part");
        self.client
            .upload_part()
            .bucket(&self.dst.bucket)
            .key(&self.dst.key)
            .upload_id(&self.upload_id)
            .part_number(part_number)
            .body(ByteStream::from(buffer))
            .send()
            .map_ok(move |p| (p, part_number))
            .boxed()
    }

    fn poll_all<T>(futures: &mut Vec<BoxFuture<T>>, cx: &mut Context) -> Vec<T> {
        let mut pending = vec![];
        let mut complete = vec![];

        while let Some(mut f) = futures.pop() {
            match Pin::new(&mut f).poll(cx) {
                Poll::Ready(result) => complete.push(result),
                Poll::Pending => pending.push(f),
            }
        }
        futures.extend(pending);
        complete
    }

    #[instrument]
    fn try_collect_complete_parts(
        complete_results: Vec<Result<(UploadPartOutput, i32), SdkError<UploadPartError>>>,
    ) -> Result<Vec<CompletedPart>, Error> {
        complete_results
            .into_iter()
            .map(|r| r.map_err(Error::other))
            .map(|r| {
                r.map(|(c, part_number)| {
                    CompletedPart::builder()
                        .set_e_tag(c.e_tag)
                        .part_number(part_number)
                        .build()
                })
            })
            .collect::<Result<Vec<_>, _>>()
    }

    #[instrument]
    fn complete_multipart_upload<'b>(
        &self,
        completed_parts: Vec<CompletedPart>,
    ) -> CompleteMultipartUploadFuture<'b> {
        self.client
            .complete_multipart_upload()
            .key(&self.dst.key)
            .bucket(&self.dst.bucket)
            .upload_id(&self.upload_id)
            .multipart_upload(
                CompletedMultipartUpload::builder()
                    .set_parts(Some(completed_parts))
                    .build(),
            )
            .send()
            .boxed()
    }

    #[instrument(skip(uploads))]
    fn check_uploads(
        uploads: &mut Vec<MultipartUploadFuture<'a>>,
        completed_parts: &mut Vec<CompletedPart>,
        cx: &mut Context,
    ) -> Result<(), Error> {
        let complete_results = AsyncMultipartUpload::poll_all(uploads, cx);
        let new_completed_parts =
            AsyncMultipartUpload::try_collect_complete_parts(complete_results)?;
        completed_parts.extend(new_completed_parts);
        Ok(())
    }
}

impl<'a> AsyncWrite for AsyncMultipartUpload<'a> {
    #[instrument(skip(self, cx, buf))]
    fn poll_write(
        mut self: Pin<&mut AsyncMultipartUpload<'a>>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        // I'm not sure how to work around borrow of two disjoint fields.
        // I had lifetime issues trying to implement Split Borrows
        event!(Level::DEBUG, "Polling write");
        let state = std::mem::replace(&mut self.state, AsyncMultipartUploadState::None);
        match state {
            AsyncMultipartUploadState::Writing {
                mut uploads,
                mut buffer,
                mut part_number,
                mut completed_parts,
            } => {
                event!(Level::DEBUG, "Polling write while Writing");
                //Poll current uploads to make space for in coming data
                AsyncMultipartUpload::check_uploads(&mut uploads, &mut completed_parts, cx)?;
                //only take enough bytes to fill remaining upload capacity
                let upload_capacity =
                    ((self.max_uploading_parts - uploads.len()) * self.part_size) - buffer.len();
                let bytes_to_write = std::cmp::min(upload_capacity, buf.len());
                // No capacity to upload
                if bytes_to_write == 0 {
                    uploads.is_empty().then(|| cx.waker().wake_by_ref());
                    self.state = AsyncMultipartUploadState::Writing {
                        uploads,
                        buffer,
                        part_number,
                        completed_parts,
                    };
                    return Poll::Pending;
                }
                buffer.extend(&buf[..bytes_to_write]);

                //keep pushing uploads until the buffer is small than the part size
                while buffer.len() >= self.part_size {
                    event!(Level::DEBUG, "Starting a new part upload");
                    let mut part = buffer.split_off(self.part_size);
                    // We want to consume the first part of the buffer and upload it to S3.
                    // The split_off call does this but it's the wrong way around.
                    // Use `mem:swap` to reverse the two variables in place.
                    std::mem::swap(&mut buffer, &mut part);
                    //Upload a new part
                    let part_upload = self.upload_part(part, part_number);
                    uploads.push(part_upload);
                    part_number += 1;
                }
                //Poll all uploads, remove complete and fetch their results.
                AsyncMultipartUpload::check_uploads(&mut uploads, &mut completed_parts, cx)?;
                //Set state back
                self.state = AsyncMultipartUploadState::Writing {
                    uploads,
                    buffer,
                    part_number,
                    completed_parts,
                };
                //Return number of bytes written from the input
                Poll::Ready(Ok(bytes_to_write))
            }
            AsyncMultipartUploadState::None => Poll::Ready(Err(Error::other(
                "Attempted to .write() when state is None",
            ))),
            _ => Poll::Ready(Err(Error::other("Attempted to .write() after .close()."))),
        }
    }

    #[instrument(skip(self, cx))]
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        //Ensure all pending uploads are completed.
        match &mut self.state {
            AsyncMultipartUploadState::Writing {
                uploads,
                completed_parts,
                ..
            } => {
                event!(Level::DEBUG, "Flushing Multipart Uploads");
                //Poll uploads and mark as completed
                AsyncMultipartUpload::check_uploads(uploads, completed_parts, cx)?;
                if uploads.is_empty() {
                    event!(Level::DEBUG, "All part uploads are complete");
                    Poll::Ready(Ok(()))
                } else {
                    event!(Level::DEBUG, "Waiting for uploads to complete");
                    //Polled futures will trigger a wake
                    Poll::Pending
                }
            }
            AsyncMultipartUploadState::None => Poll::Ready(Err(Error::other(
                "Attempted to .write() when state is None",
            ))),
            _ => Poll::Ready(Err(Error::other(
                "Attempted to .flush() writer after .close().",
            ))),
        }
    }

    #[instrument(skip(self, cx))]
    fn poll_close<'b>(
        mut self: Pin<&'b mut AsyncMultipartUpload<'a>>,
        cx: &'b mut Context<'_>,
    ) -> Poll<Result<(), Error>> {
        event!(Level::DEBUG, "Closing Multipart Uploads");
        let state = std::mem::replace(&mut self.state, AsyncMultipartUploadState::None);
        match state {
            AsyncMultipartUploadState::Writing {
                mut buffer,
                mut uploads,
                mut completed_parts,
                part_number,
            } => {
                event!(Level::DEBUG, "Creating final Part Upload");
                //make space for final upload
                AsyncMultipartUpload::check_uploads(&mut uploads, &mut completed_parts, cx)?;
                if self.max_uploading_parts - uploads.len() == 0 {
                    event!(Level::DEBUG, "Waiting for available upload capacity");
                    self.state = AsyncMultipartUploadState::Writing {
                        buffer,
                        uploads,
                        completed_parts,
                        part_number,
                    };
                    return Poll::Pending;
                }
                if !buffer.is_empty() {
                    let buff = mem::take(&mut buffer);
                    let part = self.upload_part(buff, part_number);
                    uploads.push(part);
                }
                //Poll all uploads, remove complete and fetch their results.
                AsyncMultipartUpload::check_uploads(&mut uploads, &mut completed_parts, cx)?;
                // If no remaining uploads then trigger a wake to move to next state
                uploads.is_empty().then(|| cx.waker().wake_by_ref());
                // Change state to Completing parts
                self.state = AsyncMultipartUploadState::CompletingParts {
                    uploads,
                    completed_parts,
                };
                Poll::Pending
            }
            AsyncMultipartUploadState::CompletingParts {
                uploads,
                mut completed_parts,
            } if uploads.is_empty() => {
                event!(
                    Level::DEBUG,
                    "AsyncS3Upload all parts uploaded, Completing Upload"
                );
                //Once uploads are empty change state to Completing
                // This was surprising but was needed to complete the upload.
                completed_parts.sort_by_key(|p| p.part_number());
                let completing = self.complete_multipart_upload(completed_parts);
                self.state = AsyncMultipartUploadState::Completing(completing);
                // Trigger a wake to run with new state and poll the future
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            AsyncMultipartUploadState::CompletingParts {
                mut uploads,
                mut completed_parts,
            } => {
                event!(
                    Level::DEBUG,
                    "AsyncS3Upload Waiting for All Parts to Upload"
                );
                //Poll all uploads, remove complete and fetch their results.
                AsyncMultipartUpload::check_uploads(&mut uploads, &mut completed_parts, cx)?;
                //Trigger a wake if all uploads have completed
                uploads.is_empty().then(|| cx.waker().wake_by_ref());
                self.state = AsyncMultipartUploadState::CompletingParts {
                    uploads,
                    completed_parts,
                };
                Poll::Pending
            }
            AsyncMultipartUploadState::Completing(mut fut) => {
                //Don't use ready! macro to wait for complete uploaded to be done
                //the state needs to be set back to Completing if future is Pending
                event!(Level::DEBUG, "Waiting for upload complete to finish");
                match Pin::new(&mut fut).poll(cx) {
                    Poll::Pending => {
                        self.state = AsyncMultipartUploadState::Completing(fut);
                        Poll::Pending
                    }
                    Poll::Ready(Ok(_)) => {
                        self.state = AsyncMultipartUploadState::Closed;
                        Poll::Ready(Ok(()))
                    }
                    Poll::Ready(Err(e)) => {
                        self.state = AsyncMultipartUploadState::Closed;
                        Poll::Ready(Err(Error::other(e)))
                    }
                }
            }
            AsyncMultipartUploadState::None => Poll::Ready(Err(Error::other(
                "Attempted to .close() writer with state None",
            ))),
            AsyncMultipartUploadState::Closed => Poll::Ready(Err(Error::other(
                "Attempted to .close() writer after .close().",
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_from_env;
    use crate::localstack;
    use crate::s3::test::*;
    use crate::s3::{AsyncMultipartUpload, S3Object};
    use ::function_name::named;
    use anyhow::Result;
    use aws_sdk_s3::Client;
    use bytesize::MIB;
    use futures::prelude::*;

    #[tokio::test]
    async fn test_part_size_too_small() {
        let shared_config = load_from_env().await.unwrap();
        let client = Client::new(&shared_config);
        let dst = S3Object::new("bucket", "key");
        assert!(AsyncMultipartUpload::new(&client, &dst, 0_usize, None)
            .await
            .is_err())
    }

    #[tokio::test]
    async fn test_part_size_too_big() {
        let shared_config = load_from_env().await.unwrap();
        let client = Client::new(&shared_config);
        let dst = S3Object::new("bucket", "key");
        assert!(
            AsyncMultipartUpload::new(&client, &dst, 5 * GIB as usize + 1, None)
                .await
                .is_err()
        )
    }

    #[tokio::test]
    async fn test_max_uploading_parts_is_zero() {
        let shared_config = load_from_env().await.unwrap();
        let client = Client::new(&shared_config);
        let dst = S3Object::new("bucket", "key");
        assert!(
            AsyncMultipartUpload::new(&client, &dst, 5 * MIB as usize, Some(0))
                .await
                .is_err()
        )
    }

    // *** Integration tests *** //
    //Integration tests should be in src/tests but there is tight coupling with
    //localstack which makes it hard to migrate away from this structure.
    async fn localstack_test_client() -> Client {
        localstack::test_utils::wait_for_localstack().await;
        let shared_config = crate::config::load_from_env().await.unwrap();
        let builder = aws_sdk_s3::config::Builder::from(&shared_config)
            .force_path_style(true)
            .build();
        Client::from_conf(builder)
    }

    #[tokio::test]
    #[named]
    async fn test_put_single_part() -> Result<()> {
        let client = localstack_test_client().await;
        let test_bucket = "test-multipart-bucket";
        let mut rng = seeded_rng(function_name!());
        let dst_key = gen_random_file_name(&mut rng);

        create_bucket(&client, test_bucket).await.unwrap();
        let buffer_len = MIB as usize;

        let dst = S3Object::new(test_bucket, &dst_key);
        let mut upload = AsyncMultipartUpload::new(&client, &dst, 5_usize * MIB as usize, None)
            .await
            .unwrap();
        upload.write_all(&vec![0; buffer_len]).await.unwrap();
        upload.close().await.unwrap();
        let body = fetch_bytes(&client, &dst).await.unwrap();
        assert_eq!(body.len(), buffer_len);
        Ok(())
    }

    #[tokio::test]
    #[named]
    async fn test_import_and_continue_an_upload() -> Result<()> {
        let client = localstack_test_client().await;
        let test_bucket = "test-multipart-bucket";
        let mut rng = seeded_rng(function_name!());
        let dst_key = gen_random_file_name(&mut rng);

        create_bucket(&client, test_bucket).await.unwrap();
        let buffer_len = 5_usize * MIB as usize;

        // create an existing upload to start
        let dst = S3Object::new(test_bucket, &dst_key);
        let mut upload = AsyncMultipartUpload::new(&client, &dst, 5_usize * MIB as usize, None)
            .await
            .unwrap();
        upload.write_all(&vec![0; buffer_len]).await.unwrap();
        upload.flush().await.unwrap();

        let mut resumed_upload = AsyncMultipartUpload::from(
            &client,
            upload.get_upload_id().to_string(),
            &dst,
            5_usize * MIB as usize,
            None,
        )
        .await
        .unwrap();

        resumed_upload
            .write_all(&vec![0; buffer_len])
            .await
            .unwrap();
        resumed_upload.close().await.unwrap();

        // body with 2 parts uploaded in different upload instances
        let final_body = fetch_bytes(&client, &dst).await.unwrap();
        assert_eq!(final_body.len(), 2_usize * buffer_len);
        Ok(())
    }

    #[tokio::test]
    #[named]
    async fn test_put_10mb() -> Result<()> {
        let client = localstack_test_client().await;
        let test_bucket = "test-multipart-bucket";
        let mut rng = seeded_rng(function_name!());
        let dst_key = gen_random_file_name(&mut rng);

        create_bucket(&client, test_bucket).await.unwrap();
        let buffer_len = 10 * MIB as usize;

        let dst = S3Object::new(test_bucket, &dst_key);
        let mut upload = AsyncMultipartUpload::new(&client, &dst, 5_usize * MIB as usize, None)
            .await
            .unwrap();
        upload.write_all(&vec![0; buffer_len]).await.unwrap();
        upload.close().await.unwrap();
        let body = fetch_bytes(&client, &dst).await.unwrap();
        assert_eq!(body.len(), buffer_len);
        Ok(())
    }
    #[tokio::test]
    #[named]
    async fn test_put_14mb() -> Result<()> {
        let client = localstack_test_client().await;
        let test_bucket = "test-multipart-bucket";
        let mut rng = seeded_rng(function_name!());
        let dst_key = gen_random_file_name(&mut rng);

        create_bucket(&client, test_bucket).await.unwrap();
        let buffer_len = 14 * MIB as usize;

        let dst = S3Object::new(test_bucket, &dst_key);
        let mut upload = AsyncMultipartUpload::new(&client, &dst, 5_usize * MIB as usize, None)
            .await
            .unwrap();
        upload.write_all(&vec![0; buffer_len]).await.unwrap();
        upload.close().await.unwrap();
        let body = fetch_bytes(&client, &dst).await.unwrap();
        assert_eq!(body.len(), buffer_len);
        Ok(())
    }

    #[tokio::test]
    #[named]
    async fn test_put_flush() -> Result<()> {
        let client = localstack_test_client().await;
        let test_bucket = "test-multipart-bucket";
        let mut rng = seeded_rng(function_name!());
        let dst_key = gen_random_file_name(&mut rng);

        create_bucket(&client, test_bucket).await.unwrap();
        let buffer_len = MIB as usize;

        let dst = S3Object::new(test_bucket, &dst_key);
        let mut upload = AsyncMultipartUpload::new(&client, &dst, 5_usize * MIB as usize, Some(1))
            .await
            .unwrap();
        upload.write_all(&vec![0; buffer_len]).await.unwrap();
        upload.flush().await.unwrap();
        upload.close().await.unwrap();
        let body = fetch_bytes(&client, &dst).await.unwrap();
        assert_eq!(body.len(), buffer_len);
        Ok(())
    }

    #[tokio::test]
    #[named]
    async fn test_put_double_close() -> Result<()> {
        let client = localstack_test_client().await;
        let test_bucket = "test-multipart-bucket";
        let mut rng = seeded_rng(function_name!());
        let dst_key = gen_random_file_name(&mut rng);

        create_bucket(&client, test_bucket).await.unwrap();
        let buffer_len = 100_usize;

        let dst = S3Object::new(test_bucket, &dst_key);
        let mut upload = AsyncMultipartUpload::new(&client, &dst, 5_usize * MIB as usize, Some(1))
            .await
            .unwrap();
        upload.write_all(&vec![0; buffer_len]).await.unwrap();
        upload.close().await.unwrap();
        assert!(upload.close().await.is_err());
        Ok(())
    }

    #[tokio::test]
    #[named]
    async fn test_put_write_after_close() -> Result<()> {
        let client = localstack_test_client().await;
        let test_bucket = "test-multipart-bucket";
        let mut rng = seeded_rng(function_name!());
        let dst_key = gen_random_file_name(&mut rng);

        create_bucket(&client, test_bucket).await.unwrap();
        let buffer_len = 100_usize;

        let dst = S3Object::new(test_bucket, &dst_key);
        let mut upload = AsyncMultipartUpload::new(&client, &dst, 5_usize * MIB as usize, Some(1))
            .await
            .unwrap();
        upload.write_all(&vec![0; buffer_len]).await.unwrap();
        upload.close().await.unwrap();
        assert!(upload.write_all(&vec![0; buffer_len]).await.is_err());
        Ok(())
    }

    #[tokio::test]
    #[named]
    async fn test_put_16mb_single_upload() {
        let client = localstack_test_client().await;
        let test_bucket = "test-multipart-bucket";
        let mut rng = seeded_rng(function_name!());
        let dst_key = gen_random_file_name(&mut rng);

        create_bucket(&client, test_bucket).await.unwrap();

        let dst = S3Object::new(test_bucket, &dst_key);
        let mut upload = AsyncMultipartUpload::new(&client, &dst, 5 * MIB as usize, Some(1))
            .await
            .unwrap();

        let data_len = 16 * MIB as usize;

        upload.write_all(&vec![0; data_len]).await.unwrap();
        upload.close().await.unwrap();
        let bytes = fetch_bytes(&client, &S3Object::new(test_bucket, &dst_key))
            .await
            .unwrap();
        assert_eq!(data_len, bytes.len())
    }
}
