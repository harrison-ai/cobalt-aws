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
pub mod test_utils {
    use super::*;
    use reqwest;
    use serde::Deserialize;
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    #[derive(Deserialize, Debug)]
    struct Init {
        completed: bool,
    }
    /// This function polls the <localstack_url>/_localstack/init/ready endpoint and waits for
    /// complete to be "true".
    ///
    /// We poll at one second intervals and timeout after 1 minute.
    ///
    /// This API is documented here https://docs.localstack.cloud/references/init-hooks/
    ///
    /// # Panic
    ///
    /// This function will panic on any kind of failure
    pub async fn wait_for_localstack() {
        // If tests are being run locally (e.g. not from within docker) then we
        // expect localstack to be available at localhost:4566.
        if env::var("LOCALSTACK_HOSTNAME").is_err() {
            env::set_var("LOCALSTACK_HOSTNAME", "localhost");
        }
        // Our local stack configuration is setup to run as ap-southeast-2,
        // so we explicitly set this here.
        env::set_var("AWS_DEFAULT_REGION", "ap-southeast-2");

        let uri = get_endpoint_uri().unwrap().unwrap();
        let now = Instant::now();
        loop {
            match reqwest::get(&format!("{:?}_localstack/init/ready", uri)).await {
                Err(_) => {
                    println!("Localstack URL: {:#?}", uri);
                    panic!("Unable to connect to localstack. To run localstack locally, run `docker-compose up -d`");
                }
                Ok(response) => {
                    let init: Init = response.json().await.unwrap();
                    if init.completed {
                        break;
                    }
                }
            }
            if now.elapsed().as_secs() > 60 {
                println!("Localstack URL: {:#?}", uri);
                panic!("Timed out while waiting for localstack to initialise!")
            }
            sleep(Duration::new(1, 0));
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use serial_test::serial;

    /// Run get_endpoint_uri() with a given host and port in the environment, resetting
    /// the environment to its original state afterwards.
    fn safe_get_endpoint_uri(host: Option<String>, port: Option<String>) -> Result<Option<Uri>> {
        let orig_host = env::var("LOCALSTACK_HOSTNAME");
        let orig_port = env::var("EDGE_PORT");

        match host {
            Some(host) => env::set_var("LOCALSTACK_HOSTNAME", host),
            None => env::remove_var("LOCALSTACK_HOSTNAME"),
        }

        match port {
            Some(port) => env::set_var("EDGE_PORT", port),
            None => env::remove_var("EDGE_PORT"),
        }

        let uri = get_endpoint_uri();

        match orig_host {
            Ok(host) => env::set_var("LOCALSTACK_HOSTNAME", host),
            Err(_) => env::remove_var("LOCALSTACK_HOSTNAME"),
        }
        match orig_port {
            Ok(port) => env::set_var("EDGE_PORT", port),
            Err(_) => env::remove_var("EDGE_PORT"),
        }

        uri
    }

    #[test]
    #[serial]
    fn test_get_localstack_endpoint_empty() {
        let uri = safe_get_endpoint_uri(None, None).unwrap();
        assert_eq!(uri, None);
    }

    #[test]
    #[serial]
    fn test_get_localstack_endpoint_host() {
        let uri = safe_get_endpoint_uri(Some("test_hostname".into()), None).unwrap();
        assert_eq!(uri, Some(Uri::from_static("http://test_hostname:4566")));
    }

    #[test]
    #[serial]
    fn test_get_localstack_endpoint_host_port() {
        let uri = safe_get_endpoint_uri(Some("test_hostname".into()), Some("1234".into())).unwrap();
        assert_eq!(uri, Some(Uri::from_static("http://test_hostname:1234")));
    }

    #[test]
    #[serial]
    fn test_get_localstack_endpoint_bad_uri() {
        let uri = safe_get_endpoint_uri(Some("bad:host".into()), Some("not-a-number".into()));
        match uri {
            Ok(uri) => Err(format!("Expected error, received {:?}", uri)),
            Err(e) => {
                assert!(e.to_string().contains("http"));
                Ok(())
            }
        }
        .unwrap();
    }
}
