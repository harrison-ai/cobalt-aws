//! A collection of wrappers around the [aws_sdk_athena](https://docs.rs/aws-sdk-athena/latest/aws_sdk_athena/) crate.

use anyhow::Result;
use aws_sdk_athena::config::Builder;
use aws_types::SdkConfig;

use crate::localstack;

/// Re-export of [aws_sdk_athena::client::Client](https://docs.rs/aws-sdk-athena/latest/aws_sdk_athena/client/struct.Client.html).
///
pub use aws_sdk_athena::Client;

/// Create an Athena client with LocalStack support.
///
/// # Example
///
/// ```
/// use aws_config;
/// use cobalt_aws::athena::get_client;
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
Then `aws_sdk_athena::Client::new(&shared_config)` to create the `Client`.
"#
)]
pub fn get_client(shared_config: &SdkConfig) -> Result<Client> {
    let mut builder = Builder::from(shared_config);
    if let Some(uri) = localstack::get_endpoint_uri()? {
        builder = builder.endpoint_url(uri.to_string());
    }
    Ok(Client::from_conf(builder.build()))
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
        #[allow(deprecated)]
        get_client(&config).unwrap();
    }
}
