//! s3 module docs

use anyhow::{Context, Result};
use http::Uri;
use std::env;
use std::str::FromStr;

/// Construct a LocalStack endpoint URI if the LOCALSTACK_HOSTNAME env var
/// has been set.
///
/// Ref: https://docs.localstack.cloud/localstack/configuration/
///
pub(crate) fn get_endpoint_uri() -> Result<Option<Uri>> {
    match env::var("LOCALSTACK_HOSTNAME") {
        Ok(host) => {
            let port = env::var("EDGE_PORT").unwrap_or_else(|_| "4566".to_string());
            let uri = format!("http://{}:{}", host, port);
            let uri =
                Uri::from_str(&uri).context(format!("Failed to parse LocalStack URI: {}", uri))?;
            Ok(Some(uri))
        }
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use serial_test::serial;

    #[test]
    #[serial]
    fn test_get_localstack_endpoint_empty() {
        let uri = get_endpoint_uri().unwrap();
        assert_eq!(uri, None);
    }

    #[test]
    #[serial]
    fn test_get_localstack_endpoint_host() {
        env::set_var("LOCALSTACK_HOSTNAME", "test_hostname");
        let uri = get_endpoint_uri().unwrap();
        env::remove_var("LOCALSTACK_HOSTNAME");

        assert_eq!(uri, Some(Uri::from_static("http://test_hostname:4566")));
    }

    #[test]
    #[serial]
    fn test_get_localstack_endpoint_host_port() {
        env::set_var("LOCALSTACK_HOSTNAME", "test_hostname");
        env::set_var("EDGE_PORT", "1234");
        let uri = get_endpoint_uri().unwrap();
        env::remove_var("LOCALSTACK_HOSTNAME");
        env::remove_var("EDGE_PORT");

        assert_eq!(uri, Some(Uri::from_static("http://test_hostname:1234")));
    }

    #[test]
    #[serial]
    fn test_get_localstack_endpoint_bad_uri() {
        env::set_var("LOCALSTACK_HOSTNAME", "bad:host");
        env::set_var("EDGE_PORT", "not-a-number");
        let uri = get_endpoint_uri();
        env::remove_var("LOCALSTACK_HOSTNAME");
        env::remove_var("EDGE_PORT");

        match uri {
            Ok(uri) => Err(format!("Expected error, recieved {:?}", uri)),
            Err(e) => {
                assert!(e.to_string().contains("http"));
                Ok(())
            }
        }
        .unwrap();
    }
}
