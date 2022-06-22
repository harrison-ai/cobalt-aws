//! A collection of wrappers around the [aws_sdk_s3](https://docs.rs/aws-sdk-s3/latest/aws_sdk_s3/) crate.

use anyhow::Result;
use aws_sdk_s3::error::{GetObjectError, ListObjectsV2Error};
use aws_sdk_s3::types::SdkError;
use aws_sdk_s3::{config, model, Endpoint};
use aws_types::SdkConfig;
use core::fmt::Debug;
use futures::stream;
use futures::stream::Stream;
use futures::{AsyncBufRead, TryStreamExt};

use crate::localstack;

/// Re-export of [aws_sdk_s3::client::Client](https://docs.rs/aws-sdk-s3/latest/aws_sdk_s3/client/struct.Client.html).
///
pub use aws_sdk_s3::Client;

mod async_put_object;
pub use async_put_object::AsyncPutObject;

/// Create an S3 client with LocalStack support.
///
/// # Example
///
/// ```
/// use aws_config;
/// use cobalt_aws::s3::get_client;
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
#[deprecated(
    since = "0.5.0",
    note = r#"
To create a `Client` with LocalStack support use `aws_cobalt::config::load_from_env()` to create a `SdkConfig` with LocalStack support.
Then `aws_sdk_s3::Client::new(&shared_config)` to create the `Client`.
"#
)]
pub fn get_client(shared_config: &SdkConfig) -> Result<Client> {
    let mut builder = config::Builder::from(shared_config);
    if let Some(uri) = localstack::get_endpoint_uri()? {
        builder = builder.endpoint_resolver(Endpoint::immutable(uri));
    }
    Ok(Client::from_conf(builder.build()))
}

/// Perform a bucket listing, returning a stream of results.
///
/// # Example
///
/// ```no_run
/// use aws_config;
/// use cobalt_aws::s3::{get_client, list_objects};
/// use futures::TryStreamExt;
///
/// # tokio_test::block_on(async {
/// let shared_config = aws_config::load_from_env().await;
/// let client = get_client(&shared_config).unwrap();
/// let mut objects = list_objects(&client, "my-bucket", Some("prefix".into()));
/// while let Some(item) = objects.try_next().await.unwrap() {
///     println!("{:?}", item);
/// }
/// # })
/// ```
///
/// # Implementation details
///
/// This function uses the [ListObjectsV2](https://docs.aws.amazon.com/AmazonS3/latest/API/API_ListObjectsV2.html)
/// API and performs pagination to ensure all objects are returned.
pub fn list_objects(
    client: &Client,
    bucket: impl Into<String>,
    prefix: Option<String>,
) -> impl Stream<Item = Result<model::Object, SdkError<ListObjectsV2Error>>> + Unpin {
    let req = client
        .list_objects_v2()
        .bucket(bucket)
        .set_prefix(prefix)
        .into_paginator();
    req.send()
        .map_ok(|list_objs| {
            stream::iter(
                list_objs
                    .contents
                    .unwrap_or_default() // An empty bucket comes back as None, rather than an empty vector
                    .into_iter()
                    .map(Ok),
            )
        })
        .try_flatten()
}

/// Retrieve an object from S3 as an `AsyncBufRead`.
///
/// # Example
///
/// ```no_run
/// use aws_config;
/// use cobalt_aws::s3::{get_client, get_object};
/// use futures::AsyncReadExt;
///
/// # tokio_test::block_on(async {
/// let shared_config = aws_config::load_from_env().await;
/// let client = get_client(&shared_config).unwrap();
/// let mut reader = get_object(&client, "my-bucket", "my-key").await.unwrap();
/// let mut buffer = String::new();
/// reader.read_to_string(&mut buffer).await.unwrap();
/// println!("{}", buffer);
/// # })
/// ```
pub async fn get_object(
    client: &Client,
    bucket: &str,
    key: &str,
) -> Result<impl AsyncBufRead + Debug, SdkError<GetObjectError>> {
    let req = client.get_object().bucket(bucket).key(key);
    let resp = req.send().await?;
    Ok(resp
        .body
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
        .into_async_read())
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
        let shared_config = aws_config::load_from_env().await;
        #[allow(deprecated)]
        get_client(&shared_config).unwrap();
    }
}

#[cfg(test)]
mod test_list_objects {
    use super::*;
    use aws_config;
    use futures::TryStreamExt;
    use serial_test::serial;
    use std::error::Error;
    use tokio;

    async fn localstack_test_client() -> Client {
        localstack::test_utils::wait_for_localstack().await;
        let shared_config = aws_config::load_from_env().await;
        #[allow(deprecated)]
        get_client(&shared_config).unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn test_non_existant_bucket() {
        let client = localstack_test_client().await;

        let stream = list_objects(&client, "non-existant-bucket", None);
        let e = stream.try_collect::<Vec<_>>().await.unwrap_err();
        assert!(matches!(
            e.source()
                .unwrap()
                .downcast_ref::<ListObjectsV2Error>()
                .unwrap()
                .kind,
            aws_sdk_s3::error::ListObjectsV2ErrorKind::NoSuchBucket(_)
        ))
    }

    #[tokio::test]
    #[serial]
    async fn test_empty_bucket() {
        let client = localstack_test_client().await;

        let stream = list_objects(&client, "empty-bucket", None);
        let results = stream.try_collect::<Vec<_>>().await.unwrap();
        assert_eq!(results, vec![]);
    }

    #[tokio::test]
    #[serial]
    async fn test_no_prefix() {
        let client = localstack_test_client().await;

        let stream = list_objects(&client, "test-bucket", None);
        let mut results = stream.try_collect::<Vec<_>>().await.unwrap();
        results.sort_by_cached_key(|x| x.size);
        assert_eq!(results.len(), 2503);
    }

    #[tokio::test]
    #[serial]
    async fn test_with_prefix() {
        let client = localstack_test_client().await;

        let stream = list_objects(&client, "test-bucket", Some("some-prefix".into()));
        let results = stream.try_collect::<Vec<_>>().await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0].key,
            Some("some-prefix/nested-prefix/nested.txt".into())
        );
        assert_eq!(results[0].size, 12);
        assert_eq!(results[1].key, Some("some-prefix/prefixed.txt".into()));
        assert_eq!(results[1].size, 14);
    }

    #[tokio::test]
    #[serial]
    async fn test_with_prefix_slash() {
        let client = localstack_test_client().await;

        let stream = list_objects(&client, "test-bucket", Some("some-prefix/".into()));
        let results = stream.try_collect::<Vec<_>>().await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0].key,
            Some("some-prefix/nested-prefix/nested.txt".into())
        );
        assert_eq!(results[0].size, 12);
        assert_eq!(results[1].key, Some("some-prefix/prefixed.txt".into()));
        assert_eq!(results[1].size, 14);
    }

    #[tokio::test]
    #[serial]
    async fn test_with_nested_prefix() {
        let client = localstack_test_client().await;

        let stream = list_objects(
            &client,
            "test-bucket",
            Some("some-prefix/nested-prefix".into()),
        );
        let results = stream.try_collect::<Vec<_>>().await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].key,
            Some("some-prefix/nested-prefix/nested.txt".into())
        );
        assert_eq!(results[0].size, 12);
    }

    #[tokio::test]
    #[serial]
    async fn test_with_partial_prefix() {
        let client = localstack_test_client().await;

        let stream = list_objects(&client, "test-bucket", Some("empty-pre".into()));
        let results = stream.try_collect::<Vec<_>>().await.unwrap();
        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    #[serial]
    async fn test_with_empty_prefix() {
        let client = localstack_test_client().await;

        let stream = list_objects(&client, "test-bucket", Some("empty-prefix".into()));
        let results = stream.try_collect::<Vec<_>>().await.unwrap();
        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    #[serial]
    async fn test_with_multiple_pages() {
        let client = localstack_test_client().await;

        let stream = list_objects(&client, "test-bucket", Some("multi-page".into()));
        let results = stream.try_collect::<Vec<_>>().await.unwrap();
        assert_eq!(results.len(), 2500);
    }
}
#[cfg(test)]
mod test_get_object {
    use super::*;
    use aws_config;
    use futures::AsyncReadExt;
    use serial_test::serial;
    use std::error::Error;
    use tokio;

    async fn localstack_test_client() -> Client {
        localstack::test_utils::wait_for_localstack().await;
        let shared_config = aws_config::load_from_env().await;
        #[allow(deprecated)]
        get_client(&shared_config).unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn test_non_existant_bucket() {
        let client = localstack_test_client().await;
        let e = get_object(&client, "non-existant-bucket", "my-object")
            .await
            .unwrap_err();
        let e = e
            .source()
            .unwrap()
            .downcast_ref::<GetObjectError>()
            .unwrap();

        assert!(matches!(
            e.kind,
            aws_sdk_s3::error::GetObjectErrorKind::Unhandled(_)
        ));
        assert_eq!(e.code(), Some("NoSuchBucket"));
    }

    #[tokio::test]
    #[serial]
    async fn test_non_existant_key() {
        let client = localstack_test_client().await;
        let e = get_object(&client, "test-bucket", "non-existing-object")
            .await
            .unwrap_err();
        let e = e
            .source()
            .unwrap()
            .downcast_ref::<GetObjectError>()
            .unwrap();

        assert!(matches!(
            e.kind,
            aws_sdk_s3::error::GetObjectErrorKind::NoSuchKey(_)
        ));
    }

    #[tokio::test]
    #[serial]
    async fn test_existing_key() {
        let client = localstack_test_client().await;
        let mut reader = get_object(&client, "test-bucket", "test.txt")
            .await
            .unwrap();
        let mut buffer = String::new();
        let bytes = reader.read_to_string(&mut buffer).await.unwrap();
        assert_eq!(buffer, "test data\n");
        assert_eq!(bytes, 10);
    }
}
