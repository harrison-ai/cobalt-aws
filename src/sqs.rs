//! A collection of wrappers around the [aws_sdk_sqs](https://docs.rs/aws-sdk-sqs/latest/aws_sdk_sqs/) crate.

use anyhow::Result;
use aws_sdk_sqs::types::SendMessageBatchRequestEntry;
use derive_more::{Display, From};
use futures::{Stream, StreamExt, TryFutureExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

/// Re-export of [aws_sdk_sqs::client::Client](https://docs.rs/aws-sdk-sqs/latest/aws_sdk_sqs/client/struct.Client.html).
///
pub use aws_sdk_sqs::Client;

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
/// use cobalt_aws::sqs::{Client, send_messages_concurrently};
/// use cobalt_aws::config::load_from_env;
///
/// # tokio_test::block_on(async {
/// let shared_config = load_from_env().await.unwrap();
/// let client = Client::new(&shared_config);
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
    queue_name: &SQSQueueName,
    concurrency: Option<usize>,
    msg_stream: St,
) -> Result<()> {
    if concurrency == Some(0) {
        anyhow::bail!("Zero concurrency not allowed.");
    }
    let queue_url = client
        .get_queue_url()
        .queue_name(queue_name.to_string())
        .send()
        .await?
        .queue_url
        .ok_or_else(|| anyhow::anyhow!("Failed to get queue URL for {}", queue_name))?;
    msg_stream
        .map(|msg| Ok::<_, anyhow::Error>(serde_json::to_string(&msg?)?))
        .enumerate()
        .map(|(i, s)| {
            SendMessageBatchRequestEntry::builder()
                .message_body(s?)
                .id(format!("{}", i))
                .build()
                .map_err(anyhow::Error::from)
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

/// The name of an AWS SQS queue.
///
/// The `FromStr`` implementation of this type ensures the value is a valid AWS
/// SQS name. This means it is between 1 and 80 characters, and only contains
/// alphanumberic characters, hyphens (-), and underscores (_).
///
/// Ref: https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/quotas-queues.html
#[derive(Clone, Debug, Display, From, Eq, PartialEq, Serialize, Deserialize)]
pub struct SQSQueueName(String);

impl AsRef<str> for SQSQueueName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Error, Debug)]
pub enum SQSQueueNameError {
    #[error("Min length is 1")]
    TooShort,
    #[error("Max length is 80 characters")]
    TooLong,
    #[error("The following characters are accepted: alphanumeric characters, hyphens (-), and underscores (_)")]
    InvalidCharacters,
}

const MAX_QUEUE_NAME_LENGTH: usize = 80;

impl FromStr for SQSQueueName {
    type Err = SQSQueueNameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > MAX_QUEUE_NAME_LENGTH {
            Err(SQSQueueNameError::TooLong)
        } else if s.is_empty() {
            Err(SQSQueueNameError::TooShort)
        } else if !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            Err(SQSQueueNameError::InvalidCharacters)
        } else {
            Ok(SQSQueueName(s.to_string()))
        }
    }
}

#[cfg(test)]
mod test_send_messages {
    use crate::{config::load_from_env, localstack};

    use super::*;
    use aws_sdk_sqs::{
        error::ProvideErrorMetadata, operation::get_queue_url::GetQueueUrlError,
        types::DeleteMessageBatchRequestEntry,
    };
    use futures::stream;
    use serial_test::serial;
    use tokio;

    const MAX_MESSAGES: usize = 10;

    async fn localstack_test_client() -> Client {
        localstack::test_utils::wait_for_localstack().await;
        let shared_config = load_from_env().await.unwrap();
        aws_sdk_sqs::Client::new(&shared_config)
    }

    async fn consume_queue(client: &Client, queue_name: &SQSQueueName) -> Vec<usize> {
        let queue_url = client
            .get_queue_url()
            .queue_name(queue_name.to_string())
            .send()
            .await
            .unwrap()
            .queue_url
            .unwrap();

        let mut results: Vec<usize> = vec![];
        while let Ok(x) = client
            .receive_message()
            .max_number_of_messages(MAX_MESSAGES as i32)
            .wait_time_seconds(1)
            .queue_url(&queue_url)
            .send()
            .await
        {
            match x.messages {
                Some(messages) => {
                    assert!(messages.len() <= MAX_MESSAGES);

                    let results_delete: Result<Vec<_>, _> = messages
                        .into_iter()
                        .map(|msg| {
                            results.push(msg.body.unwrap().parse().unwrap());
                            DeleteMessageBatchRequestEntry::builder()
                                .set_receipt_handle(msg.receipt_handle)
                                .set_id(msg.message_id)
                                .build()
                        })
                        .collect();
                    let results_delete =
                        results_delete.expect("Errors not expected building results_delete");

                    if results_delete.is_empty() {
                        break;
                    }

                    client
                        .delete_message_batch()
                        .queue_url(&queue_url)
                        .set_entries(Some(results_delete))
                        .send()
                        .await
                        .expect("Error deleting message batch");
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

        let queue_name = &SQSQueueName::from_str("non-existent-queue").unwrap();

        let result = send_messages_concurrently(&client, queue_name, None, item_stream).await;
        let e = result.unwrap_err();
        let e = e
            .source()
            .unwrap()
            .downcast_ref::<GetQueueUrlError>()
            .unwrap();

        assert!(matches!(e, GetQueueUrlError::QueueDoesNotExist(_)));
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

        let queue_name = &SQSQueueName::from_str("test-queue").unwrap();

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

        let queue_name = &SQSQueueName::from_str("test-queue").unwrap();

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

        let queue_name = &SQSQueueName::from_str("test-queue").unwrap();

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

        let queue_name = &SQSQueueName::from_str("test-queue").unwrap();

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

        let queue_name = &SQSQueueName::from_str("test-queue").unwrap();

        let result = send_messages_concurrently(&client, queue_name, Some(0), item_stream).await;
        let e = result.unwrap_err();

        assert_eq!(e.to_string(), "Zero concurrency not allowed.");

        let values = consume_queue(&client, queue_name).await;
        assert!(values.is_empty());
    }
}

// This module provides generators for property-based testing.
// This module provides generators for property-based testing. It is made publicly available
// under the `test-support` feature flag and during test compilation. This approach ensures
// that external modules and other crates can optionally include and utilize these generators
// for their testing purposes when the `test-utils` feature is enabled or during the crate's
// own test runs. This feature-guarded accessibility helps maintain clean separation between
// test utilities and production code while enabling code reuse in testing contexts.
#[cfg(any(test, feature = "test-utils"))]
mod test_support {
    use super::*;
    use proptest::prelude::*;
    use proptest::strategy::{BoxedStrategy, Strategy};

    // Arbitrary implementation for SQSQueueName for testing
    impl Arbitrary for SQSQueueName {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            let pattern = "[a-zA-Z0-9_-]{1,80}";
            proptest::string::string_regex(pattern)
                .expect("Invalid regex pattern for SQSQueueName")
                .prop_map(|s| SQSQueueName::from_str(&s).unwrap())
                .boxed()
        }
    }

    #[allow(dead_code)]
    pub fn sqs_name_arbitrary_invalid() -> BoxedStrategy<String> {
        let too_short = Just("".to_string()); // Too short

        let too_long = "a".repeat(MAX_QUEUE_NAME_LENGTH + 1); // Too long
        let too_long = Just(too_long);

        let invalid_chars = "[*?%!]{1,10}"; // Contains invalid characters
        let invalid_chars = proptest::string::string_regex(invalid_chars)
            .expect("Invalid regex pattern for generating invalid SQSQueueName");

        prop_oneof![too_short, too_long, invalid_chars].boxed()
    }
}

#[cfg(test)]
mod prop_tests {
    /**
    Module containing property-based tests for the paraent module.

    In these tests, `prop_assert!` is used extensively for a few key reasons:

    1. **Integration with `proptest`**: Unlike `assert!` or `assert_eq!`, `prop_assert!`
       and its variants (e.g., `prop_assert_eq!`) are designed to work seamlessly
       within the `proptest` framework. They handle failure reporting in a way that
       integrates with `proptest`'s test case reduction mechanisms, making it easier
       to diagnose and understand failures.

    2. **Test Case Reduction**: When a `prop_assert!` fails, `proptest` attempts to
       "shrink" the input data to the smallest case that still causes the assertion
       to fail. This simplification process is crucial for debugging and is a major
       advantage of property-based testing. `prop_assert!` ensures that shrinking
       behavior works correctly.

    3. **Custom Failure Messages**: Like `assert!`, `prop_assert!` allows for custom
       failure messages. This feature is particularly useful in complex tests where
       the default error message may not provide enough context about the failure.

    Using `prop_assert!` correctly is essential for leveraging the full power of
    property-based testing with `proptest`.
    */
    use super::test_support::*;
    use crate::sqs::SQSQueueName;
    use assert_matches::assert_matches;
    use proptest::prelude::*;
    use std::str::FromStr;

    proptest! {

        // Tests that serialization and deserialization of `SQSQueueName` is symmetric,
        // ensuring that any `SQSQueueName` can be round-tripped to JSON and back without loss.
        #[test]
        fn sqs_name_round_trip_test(queue_name: SQSQueueName) {
            let serialized = serde_json::to_string(&queue_name).expect("SQS queue name should be valid");
            let deserialized:  SQSQueueName = serde_json::from_str(&serialized).expect("Input json should be valid");
            prop_assert_eq!(deserialized, queue_name);
        }

        //Invalid SQSNames should fail
        #[test]
        fn test_invalid_sqs_name(invalid_str in sqs_name_arbitrary_invalid()) {
            assert_matches!(SQSQueueName::from_str(&invalid_str), Err(_))
        }

    }
}
