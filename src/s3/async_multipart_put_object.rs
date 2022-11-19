//! Provides ways of interacting with objects in S3.

use anyhow::Context as _;
use aws_sdk_s3::error::{CompleteMultipartUploadError, UploadPartError};
use aws_sdk_s3::model::CompletedMultipartUpload;
use aws_sdk_s3::model::CompletedPart;
use aws_sdk_s3::model::ObjectCannedAcl;
use aws_sdk_s3::output::{CompleteMultipartUploadOutput, UploadPartOutput};
use aws_sdk_s3::types::{ByteStream, SdkError};
use bytesize::{GIB, MIB};
use futures::future::BoxFuture;
use futures::io::{Error, ErrorKind};
use futures::task::{Context, Poll};
use futures::{ready, AsyncWrite, Future, FutureExt, TryFutureExt};
use std::mem;
use std::pin::Pin;

use aws_sdk_s3::Client;
use tracing::{event, instrument, Level};

use crate::s3::S3Object;

/// Convenience wrapper for boxed future
type MultipartUploadFuture<'a> =
    BoxFuture<'a, Result<(UploadPartOutput, i32), SdkError<UploadPartError>>>;
/// Convenience wrapper for boxed future
type CompleteMultipartUploadFuture<'a> =
    BoxFuture<'a, Result<CompleteMultipartUploadOutput, SdkError<CompleteMultipartUploadError>>>;

/// Holds state for the [AsyncMultipartUpload]
enum AsyncMultipartUploadState<'a> {
    ///Bytes are being written
    Writing {
        /// Multipart Uploads that are running
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
        uploads: Vec<MultipartUploadFuture<'a>>,
        completed_parts: Vec<CompletedPart>,
    },
    /// All parts have been uploaded and the CompleteMultipart is returning.
    Completing(CompleteMultipartUploadFuture<'a>),
    // We have completed writing to S3.
    Closed,
}

/// Configuration for the AsyncMultipartUpload which
/// is separate from the state.
/// This was to work around borrowing of two disjoint fields
/// allowing this structure to be cloned.
#[derive(Clone, Debug)]
struct AsyncMultipartUploadConfig<'a> {
    client: &'a Client,
    dst: S3Object,
    upload_id: String,
    part_size: usize,
    max_uploading_parts: usize,
}

/// A implementation of [AsyncWrite] for S3 objects using multipart uploads.
/// By using multipart uploads constant memory usage can be achieved.
///
/// ## Note
/// On failure the multipart upload is not aborted. It is up to the
/// caller to call the S3 `abortMultipartUpload` API when required.
pub struct AsyncMultipartUpload<'a> {
    config: AsyncMultipartUploadConfig<'a>,
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
    /// * `bucket`              - Bucket to write the object into.
    /// * `key`                 - Key to write the object into.
    /// * `part_size`           - How large, in bytes, each part should be.
    /// * `max_uploading_parts` - How many parts to upload concurrently.
    #[instrument(skip(client))]
    pub async fn new(
        client: &'a Client,
        dst: S3Object,
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

        if max_uploading_parts.unwrap_or(DEFAULT_MAX_UPLOADING_PARTS) == 0 {
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
            config: AsyncMultipartUploadConfig {
                client,
                dst,
                upload_id: upload_id.into(),
                part_size,
                max_uploading_parts: max_uploading_parts.unwrap_or(DEFAULT_MAX_UPLOADING_PARTS),
            },
            state: AsyncMultipartUploadState::Writing {
                uploads: vec![],
                buffer: Vec::with_capacity(part_size),
                part_number: 1,
                completed_parts: vec![],
            },
        })
    }

    #[instrument(skip(buffer))]
    fn upload_part<'b>(
        config: &AsyncMultipartUploadConfig,
        buffer: Vec<u8>,
        part_number: i32,
    ) -> MultipartUploadFuture<'b> {
        event!(Level::DEBUG, "Uploading Part");
        config
            .client
            .upload_part()
            .bucket(&config.dst.bucket)
            .key(&config.dst.key)
            .upload_id(&config.upload_id)
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
            .map(|r| r.map_err(|e| Error::new(ErrorKind::Other, e)))
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
        config: &AsyncMultipartUploadConfig,
        completed_parts: Vec<CompletedPart>,
    ) -> CompleteMultipartUploadFuture<'b> {
        config
            .client
            .complete_multipart_upload()
            .key(&config.dst.key)
            .bucket(&config.dst.bucket)
            .upload_id(&config.upload_id)
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
        let config = self.config.clone();
        match &mut self.state {
            AsyncMultipartUploadState::Writing {
                uploads,
                buffer,
                part_number,
                completed_parts,
            } => {
                event!(Level::DEBUG, "Polling write while Writing");
                //Poll current uploads to make space for in coming data
                AsyncMultipartUpload::check_uploads(uploads, completed_parts, cx)?;
                //only take enough bytes to fill remaining upload capacity
                let upload_capacity = ((config.max_uploading_parts - uploads.len())
                    * config.part_size)
                    - buffer.len();
                let bytes_to_write = std::cmp::min(upload_capacity, buf.len());
                // No capacity to upload
                if bytes_to_write == 0 {
                    uploads.is_empty().then(|| cx.waker().wake_by_ref());
                    return Poll::Pending;
                }
                buffer.extend(&buf[..bytes_to_write]);

                //keep pushing uploads until the buffer is small than the part size
                while buffer.len() >= config.part_size {
                    event!(Level::DEBUG, "Starting a new part upload");
                    let mut part = buffer.split_off(config.part_size);
                    // We want to consume the first part of the buffer and upload it to S3.
                    // The split_off call does this but it's the wrong way around.
                    // Use `mem:swap` to reverse the two variables in place.
                    std::mem::swap(buffer, &mut part);
                    //Upload a new part
                    let part_upload =
                        AsyncMultipartUpload::upload_part(&config, part, *part_number);
                    uploads.push(part_upload);
                    *part_number += 1;
                }
                //Poll all uploads, remove complete and fetch their results.
                AsyncMultipartUpload::check_uploads(uploads, completed_parts, cx)?;
                //Return number of bytes written from the input
                Poll::Ready(Ok(bytes_to_write))
            }
            _ => Poll::Ready(Err(Error::new(
                ErrorKind::Other,
                "Attempted to .write() after .close().",
            ))),
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
                    //Assume that polled futures will trigger a wake
                    Poll::Pending
                }
            }
            _ => Poll::Ready(Err(Error::new(
                ErrorKind::Other,
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
        let config = self.config.clone();
        match &mut self.state {
            AsyncMultipartUploadState::Writing {
                buffer,
                uploads,
                completed_parts,
                part_number,
            } => {
                event!(Level::DEBUG, "Creating final Part Upload");
                //make space for final upload
                AsyncMultipartUpload::check_uploads(uploads, completed_parts, cx)?;
                if config.max_uploading_parts - uploads.len() == 0 {
                    event!(Level::DEBUG, "Waiting for available upload capacity");
                    return Poll::Pending;
                }
                if !buffer.is_empty() {
                    let buff = mem::take(buffer);
                    let part = AsyncMultipartUpload::upload_part(&config, buff, *part_number);
                    uploads.push(part);
                }
                //Poll all uploads, remove complete and fetch their results.
                AsyncMultipartUpload::check_uploads(uploads, completed_parts, cx)?;
                // If no remaining uploads then trigger a wake to move to next state
                uploads.is_empty().then(|| cx.waker().wake_by_ref());
                // Change state to Completing parts
                self.state = AsyncMultipartUploadState::CompletingParts {
                    uploads: mem::take(uploads),
                    completed_parts: mem::take(completed_parts),
                };
                Poll::Pending
            }
            AsyncMultipartUploadState::CompletingParts {
                uploads,
                completed_parts,
            } if uploads.is_empty() => {
                event!(
                    Level::DEBUG,
                    "AsyncS3Upload all parts uploaded, Completing Upload"
                );
                //Once uploads are empty change state to Completing
                let mut completed_parts = mem::take(completed_parts);
                // This was surprising but was needed to complete the upload.
                completed_parts.sort_by_key(|p| p.part_number());
                let completing =
                    AsyncMultipartUpload::complete_multipart_upload(&config, completed_parts);
                self.state = AsyncMultipartUploadState::Completing(completing);
                // Trigger a wake to run with new state and poll the future
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            AsyncMultipartUploadState::CompletingParts {
                uploads,
                completed_parts,
            } => {
                event!(
                    Level::DEBUG,
                    "AsyncS3Upload Waiting for All Parts to Upload"
                );
                //Poll all uploads, remove complete and fetch their results.
                AsyncMultipartUpload::check_uploads(uploads, completed_parts, cx)?;
                //Trigger a wake if all uploads have completed
                uploads.is_empty().then(|| cx.waker().wake_by_ref());
                Poll::Pending
            }
            AsyncMultipartUploadState::Completing(fut) => {
                //use ready! macro to wait for complete uploaded to be done
                //ready! is like the ? but for Poll objects returning `Polling` if not Ready
                event!(Level::DEBUG, "Waiting for upload complete to finish");
                let result = ready!(Pin::new(fut).poll(cx))
                    .map(|_| ())
                    .map_err(|e| Error::new(ErrorKind::Other, e)); //set state to closed
                self.state = AsyncMultipartUploadState::Closed;
                Poll::Ready(result)
            }
            AsyncMultipartUploadState::Closed => Poll::Ready(Err(Error::new(
                ErrorKind::Other,
                "Attempted to .close() writer after .close().",
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_part_size_too_small() {
        let shared_config = aws_config::load_from_env().await;
        let client = aws_sdk_s3::Client::new(&shared_config);
        let dst = S3Object::new("bucket", "key");
        assert!(AsyncMultipartUpload::new(&client, dst, 0_usize, None)
            .await
            .is_err())
    }

    #[tokio::test]
    async fn test_part_size_too_big() {
        let shared_config = aws_config::load_from_env().await;
        let client = aws_sdk_s3::Client::new(&shared_config);
        let dst = S3Object::new("bucket", "key");
        assert!(
            AsyncMultipartUpload::new(&client, dst, 5 * GIB as usize + 1, None)
                .await
                .is_err()
        )
    }

    #[tokio::test]
    async fn test_max_uploading_parts_is_zero() {
        let shared_config = aws_config::load_from_env().await;
        let client = aws_sdk_s3::Client::new(&shared_config);
        let dst = S3Object::new("bucket", "key");
        assert!(
            AsyncMultipartUpload::new(&client, dst, 5 * MIB as usize, Some(0))
                .await
                .is_err()
        )
    }
}
