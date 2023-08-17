use aws_sdk_s3::primitives::SdkBody;
use http::Response;

/// Convenience wrapper to handle http response
pub(crate) type SdkError<E> = aws_smithy_http::result::SdkError<E, Response<SdkBody>>;
