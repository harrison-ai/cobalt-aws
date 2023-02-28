//! A collection of wrappers around the [aws_types::SdkConfig](https://docs.rs/aws-types/0.13.0/aws_types/sdk_config/struct.SdkConfig.html) object.

use anyhow::Result;
use aws_types::SdkConfig;

use crate::localstack;

/// Create a shared `SdkConfig` with LocalStack support.
///
/// # Example
///
/// ```
/// use aws_config;
/// use cobalt_aws::config::load_from_env;
/// use cobalt_aws::s3::Client;
///
/// # tokio_test::block_on(async {
/// let shared_config = load_from_env().await.unwrap();
/// let client = Client::new(&shared_config);
/// # })
/// ```
///
/// ## LocalStack
///
/// This shared `SdkConfig` supports creating clients that run on [LocalStack](https://localstack.cloud/).
///
/// If you use this config to create a client from within a Lambda function that is running on
/// LocalStack, it will automatically setup the correct endpoint.
///
/// If you use this config to create a client from outside of LocalStack but want to communicate
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
pub async fn load_from_env() -> Result<SdkConfig> {
    let mut shared_config = aws_config::from_env();
    if let Some(uri) = localstack::get_endpoint_uri()? {
        shared_config = shared_config.endpoint_url(uri.to_string());
    }
    Ok(shared_config.load().await)
}

#[cfg(test)]
mod test {
    use super::*;

    use serial_test::serial;
    use tokio;

    #[tokio::test]
    #[serial]
    async fn test_load_from_client() {
        load_from_env().await.unwrap();
    }
}
