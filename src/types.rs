use aws_sdk_s3::primitives::SdkBody;
use aws_smithy_runtime_api::http::Response;

/// Convenience wrapper to handle http response
pub(crate) type SdkError<E> = aws_sdk_s3::error::SdkError<E, Response<SdkBody>>;
