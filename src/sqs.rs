//! A collection of wrappers around the [aws_sdk_sqs](https://docs.rs/aws-sdk-sqs/latest/aws_sdk_sqs/) crate.

use anyhow::Result;
use aws_sdk_sqs::model::SendMessageBatchRequestEntry;
use aws_sdk_sqs::{config, Endpoint};
use futures::{Stream, StreamExt, TryFutureExt, TryStreamExt};

use crate::localstack;

/// Re-export of [aws_sdk_sqs::client::Client](https://docs.rs/aws-sdk-sqs/latest/aws_sdk_sqs/client/struct.Client.html).
///
pub use aws_sdk_sqs::Client;

/// Create an SQS client with LocalStack support.
///
/// # Example
///
/// ```
/// use aws_config;
/// use cobalt_aws::sqs::get_client;
///
/// # tokio_test::block_on(async {
/// let shared_config = aws_config::load_from_env().await;
/// let client = get_client(&shared_config).unwrap();
/// # })
/// ```
///
/// ## LocalStack
///
/// This client supports running on [LocalStack](https://localstack.cloud/).
///
/// If you're using this client from within a Lambda function that is running on
/// LocalStack, it will automatically setup the correct endpoint.
///
/// If you're using this client from outside of LocalStack but want to communicate
/// with a LocalStack instance, then set the environment variable `LOCALSTACK_HOSTNAME`:
///
/// ```shell
/// $ export LOCALSTACK_HOSTNAME=localhost
/// ```
///
/// You can also optionally set the `EDGE_PORT` variable if you need something other
/// than the default of `4566`.
///
/// See the [LocalStack configuration docs](https://docs.localstack.cloud/localstack/configuration/) for more info.
///
/// ## Errors
///
/// An error will be returned if `LOCALSTACK_HOSTNAME` is set and a valid URI cannot be constructed.
///
pub fn get_client(shared_config: &aws_config::Config) -> Result<Client> {
    let mut builder = config::Builder::from(shared_config);
    if let Some(uri) = localstack::get_endpoint_uri()? {
        builder = builder.endpoint_resolver(Endpoint::immutable(uri));
    }
    Ok(Client::from_conf(builder.build()))
}

const BATCH_SIZE: usize = 10;

/// Send message to a queue from a stream with concurrent invocations of `SendMessageBatch`.
///
/// This function retrieves items from the stream until it has exhausted. Any errors
/// in the stream, or while processing the stream, are returned as soon as they are encountered.
///
/// If `concurrency` is `None` then message batches are sent sequentially. A `concurrency` value of
/// zero, `Some(0)`, is not allowed and will result in an error.
///
/// # Example
///
/// ```no_run
/// use aws_config;
/// use futures::stream;
/// use cobalt_aws::sqs::{get_client, send_messages_concurrently};
///
/// # tokio_test::block_on(async {
/// let shared_config = aws_config::load_from_env().await;
/// let client = get_client(&shared_config).unwrap();
///
/// let messages = stream::iter(vec![Ok("Hello"), Ok("world")]);
/// let queue_name = "MyQueue";
///
/// // Send up to 4 concurrent API requests at once.
/// send_messages_concurrently(&client, queue_name, Some(4), messages).await.unwrap();
/// # })
/// ```
///
/// # Implementation details
///
/// This function uses the [SendMessageBatch](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/APIReference/API_SendMessageBatch.html)
/// API to send message in batches of 10, which is the maximum allowed batch size.
pub async fn send_messages_concurrently<Msg: serde::Serialize, St: Stream<Item = Result<Msg>>>(
    client: &Client,
    queue_name: &str,
    concurrency: Option<usize>,
    msg_stream: St,
) -> Result<()> {
    if concurrency == Some(0) {
        anyhow::bail!("Zero concurrency not allowed.");
    }
    let queue_url = client
        .get_queue_url()
        .queue_name(queue_name)
        .send()
        .await?
        .queue_url
        .ok_or_else(|| anyhow::anyhow!("Failed to get queue URL for {}", queue_name))?;
    msg_stream
        .map(|msg| Ok::<_, anyhow::Error>(serde_json::to_string(&msg?)?))
        .enumerate()
        .map(|(i, s)| {
            Ok::<_, anyhow::Error>(
                SendMessageBatchRequestEntry::builder()
                    .message_body(s?)
                    .id(format!("{}", i))
                    .build(),
            )
        })
        .try_chunks(BATCH_SIZE)
        .map_err(anyhow::Error::from)
        .inspect_ok(|entries| tracing::debug!("Sending message batch: {:#?}", entries))
        .map_ok(|entries| {
            client
                .send_message_batch()
                .queue_url(&queue_url)
                .set_entries(Some(entries))
                .send()
                .map_err(anyhow::Error::from)
                .map_ok(|_| ())
        })
        .try_buffered(concurrency.unwrap_or(1))
        .try_collect::<()>()
        .await
}

#[cfg(test)]
mod test {
    use super::*;

    use aws_config;
    use serial_test::serial;
    use tokio;

    #[tokio::test]
    #[serial]
    async fn test_get_client() {
        let config = aws_config::load_from_env().await;
        get_client(&config).unwrap();
    }
}

#[cfg(test)]
mod test_send_messages_concurrently {
    use super::*;
    use aws_config;
    use aws_sdk_sqs::error::GetQueueUrlError;
    use futures::stream;
    use serial_test::serial;
    use tokio;

    async fn localstack_test_client() -> Client {
        localstack::test_utils::wait_for_localstack().await;
        let shared_config = aws_config::load_from_env().await;
        get_client(&shared_config).unwrap()
    }

    async fn consume_queue(client: &Client, queue_name: &str) -> Vec<usize> {
        let queue_url = client
            .get_queue_url()
            .queue_name(queue_name)
            .send()
            .await
            .unwrap()
            .queue_url
            .unwrap();

        // FIXME: use batch operations
        let mut results: Vec<usize> = vec![];
        while let Ok(x) = client
            .receive_message()
            .wait_time_seconds(1)
            .queue_url(&queue_url)
            .send()
            .await
        {
            match x.messages {
                Some(ref messages) => {
                    assert_eq!(messages.len(), 1);
                    let message = &messages[0];
                    results.push(message.body.as_ref().unwrap().parse().unwrap());
                    client
                        .delete_message()
                        .queue_url(&queue_url)
                        .receipt_handle(message.receipt_handle.as_ref().unwrap())
                        .send()
                        .await
                        .unwrap();
                }
                None => break,
            }
        }
        results.sort_unstable();
        results
    }

    #[tokio::test]
    #[serial]
    async fn test_non_existent_queue() {
        let client = localstack_test_client().await;

        let item_stream = stream::iter(vec![Ok::<u32, _>(1), Ok(2), Ok(3)]);

        let queue_name = "non-existent-queue";

        let result = send_messages_concurrently(&client, queue_name, None, item_stream).await;
        let e = result.unwrap_err();
        let e = e
            .source()
            .unwrap()
            .downcast_ref::<GetQueueUrlError>()
            .unwrap();

        assert!(matches!(
            e.kind,
            aws_sdk_sqs::error::GetQueueUrlErrorKind::QueueDoesNotExist(_)
        ));
        assert_eq!(e.code(), Some("AWS.SimpleQueueService.NonExistentQueue"));
    }

    #[tokio::test]
    #[serial]
    async fn test_item_stream_error() {
        let client = localstack_test_client().await;

        let item_stream = stream::iter(vec![
            Ok::<u32, _>(1),
            Ok(2),
            Err(anyhow::anyhow!("some error")),
            Ok(3),
        ]);

        let queue_name = "test-queue";

        let result = send_messages_concurrently(&client, queue_name, None, item_stream).await;
        let e = result.unwrap_err();
        assert_eq!(e.to_string(), "some error");

        let values = consume_queue(&client, queue_name).await;
        assert!(values.is_empty());
    }

    #[tokio::test]
    #[serial]
    async fn test_less_than_batch_size() {
        let client = localstack_test_client().await;

        let item_stream = stream::iter((0..5).map(Ok));

        let queue_name = "test-queue";

        let result = send_messages_concurrently(&client, queue_name, None, item_stream).await;
        result.unwrap();

        let values = consume_queue(&client, queue_name).await;
        assert_eq!(values, (0..5).collect::<Vec<_>>());
    }

    #[tokio::test]
    #[serial]
    async fn test_more_than_batch_size() {
        let client = localstack_test_client().await;

        let item_stream = stream::iter((0..25).map(Ok));

        let queue_name = "test-queue";

        let result = send_messages_concurrently(&client, queue_name, None, item_stream).await;
        result.unwrap();

        let values = consume_queue(&client, queue_name).await;
        assert_eq!(values, (0..25).collect::<Vec<_>>());
    }

    #[tokio::test]
    #[serial]
    async fn test_concurrent() {
        let client = localstack_test_client().await;

        let item_stream = stream::iter((0..105).map(Ok));

        let queue_name = "test-queue";

        let result = send_messages_concurrently(&client, queue_name, Some(5), item_stream).await;
        result.unwrap();

        let values = consume_queue(&client, queue_name).await;
        assert_eq!(values, (0..105).collect::<Vec<_>>());
    }

    #[tokio::test]
    #[serial]
    async fn test_zero_concurrency() {
        let client = localstack_test_client().await;

        let item_stream = stream::iter((0..105).map(Ok));

        let queue_name = "test-queue";

        let result = send_messages_concurrently(&client, queue_name, Some(0), item_stream).await;
        let e = result.unwrap_err();

        assert_eq!(e.to_string(), "Zero concurrency not allowed.");

        let values = consume_queue(&client, queue_name).await;
        assert!(values.is_empty());
    }
}
