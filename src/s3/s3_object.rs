use std::str::FromStr;

use anyhow::{bail, Context, Error, Result};
use serde::{Deserialize, Serialize};
use url::Url;

/// A bucket key pair for a S3Object, with conversion from S3 urls.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct S3Object {
    /// The bucket the object is in.
    pub bucket: String,
    /// The key the in the bucket for the object.
    pub key: String,
}

impl S3Object {
    /// Create a new [S3Object] using anything which can be
    /// treated as [&str].  Any leading `/` will be trimmed from
    /// the key.  No validation is done against the bucket or key
    /// to ensure they meet the AWS requirements.
    pub fn new(bucket: impl AsRef<str>, key: impl AsRef<str>) -> Self {
        S3Object {
            bucket: bucket.as_ref().to_owned(),
            key: key.as_ref().trim_start_matches('/').to_owned(),
        }
    }
}

/// Convert from an [Url] into a [S3Object]. The scheme
/// must be `s3` and the `path` must not be empty.
impl TryFrom<Url> for S3Object {
    type Error = Error;

    fn try_from(value: Url) -> Result<Self, Self::Error> {
        if value.scheme() != "s3" {
            bail!("S3 URL must have a scheme of s3")
        }
        let bucket = value.host_str().context("S3 URL must have host")?;
        let key = value.path();

        if key.is_empty() {
            bail!("S3 URL must have a path")
        }
        Ok(S3Object::new(bucket, key))
    }
}

/// Convert from a [&str] into a [S3Object].
/// The [&str] must be a valid `S3` [Url].
impl TryFrom<&str> for S3Object {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse::<Url>()?.try_into()
    }
}

/// Covert from [String] into a [S3Object].
/// The [String] must be a valid `S3` [Url].
impl TryFrom<String> for S3Object {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse::<Url>()?.try_into()
    }
}

/// Convert from [&str] into a [S3Object].
/// The [&str] must be a valid `S3` [Url].
impl FromStr for S3Object {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse::<Url>()?.try_into()
    }
}

/// Converts from a [S3Object] into a [Url].
/// If the [S3Object] holds an invalid path or
/// domain the conversion will fail.
impl TryFrom<S3Object> for Url {
    type Error = url::ParseError;
    fn try_from(obj: S3Object) -> std::result::Result<Self, Self::Error> {
        Url::parse(&format!("s3://{}/{}", obj.bucket, obj.key))
    }
}

/// Converts from a [S3Object] into a [Url].
/// If the [S3Object] holds an invalid path or
/// domain the conversion will fail.
impl TryFrom<&S3Object> for Url {
    type Error = url::ParseError;
    fn try_from(obj: &S3Object) -> std::result::Result<Self, Self::Error> {
        Url::parse(&format!("s3://{}/{}", obj.bucket, obj.key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s3_tryfrom() {
        let bucket = "test-bucket.to_owned()";
        let key = "test-key";
        let url: url::Url = format!("s3://{bucket}/{key}")
            .parse()
            .expect("Expected successful URL parsing");
        let obj: S3Object = url.try_into().expect("Expected successful URL conversion");
        assert_eq!(bucket, obj.bucket);
        assert_eq!(key, obj.key);
    }

    #[test]
    fn test_s3_tryfrom_no_path() {
        let url: url::Url = "s3://test-bucket"
            .parse()
            .expect("Expected successful URL parsing");
        let result: Result<S3Object> = url.try_into();
        assert!(result.is_err())
    }

    #[test]
    fn test_s3_tryfrom_file_url() {
        let url: url::Url = "file://path/to/file"
            .parse()
            .expect("Expected successful URL parsing");
        let result: Result<S3Object> = url.try_into();
        assert!(result.is_err())
    }
}
