use aws_sdk_s3::error::PutObjectError;
use aws_sdk_s3::model::ObjectCannedAcl;
use aws_sdk_s3::output::PutObjectOutput;
use aws_sdk_s3::types::SdkError;
use futures::io::{Error, ErrorKind};
use futures::task::{Context, Poll};
use futures::{AsyncWrite, Future};
use std::mem;
use std::pin::Pin;

use crate::s3::Client;

enum PutObjectState<'a> {
    Writing,
    Closing(Pin<Box<dyn Future<Output = Result<PutObjectOutput, SdkError<PutObjectError>>> + 'a>>),
    Closed,
}

/// Implements the `AsyncWrite` trait and writes data to S3 using the `put_object` API.
///
/// # Example
///
/// ```no_run
/// use aws_config;
/// use cobalt_aws::s3::{get_client, AsyncPutObject};
/// use futures::AsyncWriteExt;
///
/// # tokio_test::block_on(async {
/// let shared_config = aws_config::load_from_env().await;
/// let client = get_client(&shared_config).unwrap();
///
/// let mut writer = AsyncPutObject::new(&client, "my-bucket", "my-key");
/// writer.write_all(b"File contents").await.unwrap();
///
/// // The contents are pushed to S3 when the `.close()` method is called.
/// writer.close().await.unwrap();
/// # })
/// ```
pub struct AsyncPutObject<'a> {
    client: &'a Client,
    key: String,
    bucket: String,
    buf: Vec<u8>,
    state: PutObjectState<'a>,
}

impl<'a> AsyncPutObject<'a> {
    /// Create a new `AsyncPutObject` which will write data to the given `bucket` and `key`.
    pub fn new(client: &'a Client, bucket: &str, key: &str) -> Self {
        AsyncPutObject {
            client,
            key: key.into(),
            bucket: bucket.into(),
            buf: vec![],
            state: PutObjectState::Writing,
        }
    }
}

impl<'a> AsyncWrite for AsyncPutObject<'a> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        match self.state {
            PutObjectState::Writing => {
                self.buf.extend(buf);
                Poll::Ready(Ok(buf.len()))
            }
            _ => Poll::Ready(Err(Error::new(ErrorKind::Other, "bad poll_write"))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        match self.state {
            PutObjectState::Writing => Poll::Ready(Ok(())),
            _ => Poll::Ready(Err(Error::new(ErrorKind::Other, "bad poll_flush"))),
        }
    }

    fn poll_close<'b>(
        mut self: Pin<&'b mut AsyncPutObject<'a>>,
        cx: &'b mut Context<'_>,
    ) -> Poll<Result<(), Error>> {
        match self.state {
            PutObjectState::Writing => {
                // Create the put_object future and transition to
                // the Closing state.
                let fut = self
                    .client
                    .put_object()
                    .bucket(&self.bucket)
                    .key(&self.key)
                    .body(mem::take(&mut self.buf).into())
                    .acl(ObjectCannedAcl::BucketOwnerFullControl)
                    .send();
                self.state = PutObjectState::Closing(Box::pin(fut));
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            PutObjectState::Closing(ref mut fut) => {
                match Pin::new(fut).poll(cx) {
                    Poll::Ready(x) => {
                        self.state = PutObjectState::Closed;
                        match x {
                            Ok(_) => Poll::Ready(Ok(())),
                            Err(e) => Poll::Ready(Err(Error::new(ErrorKind::Other, e))),
                        }
                    }
                    Poll::Pending => {
                        // Tell the async executor that we're ready to try again whenever it is!
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                }
            }
            PutObjectState::Closed => {
                Poll::Ready(Err(Error::new(ErrorKind::Other, "bad poll_close")))
            }
        }
    }
}

#[cfg(test)]
mod test_async_put_object {
    use super::*;
    use crate::localstack;
    use crate::s3::get_client;
    use aws_config;
    use futures::{AsyncReadExt, AsyncWriteExt};
    use serial_test::serial;
    use std::error::Error;
    use tokio;

    async fn localstack_test_client() -> Client {
        localstack::test_utils::wait_for_localstack().await;
        let shared_config = aws_config::load_from_env().await;
        get_client(&shared_config).unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn test_non_existant_bucket() {
        let client = localstack_test_client().await;
        let mut writer = AsyncPutObject::new(&client, "non-existant-bucket", "my-object");
        writer.write_all(b"File contents").await.unwrap();
        let e = writer.close().await.unwrap_err();
        let e = e
            .source()
            .unwrap()
            .downcast_ref::<PutObjectError>()
            .unwrap();

        assert!(matches!(
            e.kind,
            aws_sdk_s3::error::PutObjectErrorKind::Unhandled(_)
        ));
        assert_eq!(e.code(), Some("NoSuchBucket"));
    }

    #[tokio::test]
    #[serial]
    async fn test_write() {
        let client = localstack_test_client().await;
        let mut writer = AsyncPutObject::new(&client, "test-bucket", "test-output.txt");
        writer.write_all(b"File contents").await.unwrap();
        writer.close().await.unwrap();

        // Check contents
        let mut buffer = String::new();
        let mut reader = crate::s3::get_object(&client, "test-bucket", "test-output.txt")
            .await
            .unwrap();
        reader.read_to_string(&mut buffer).await.unwrap();
        assert_eq!(buffer, "File contents");

        // Clean up
        client
            .delete_object()
            .bucket("test-bucket")
            .key("test-output.txt")
            .send()
            .await
            .unwrap();
    }
}
