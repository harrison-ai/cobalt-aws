use aws_sdk_s3::primitives::SdkBody;
use aws_smithy_http::result::SdkError;
use http::Response;

/// Convenience wrapper to handle http response
pub(crate) type HttpResponseSdkError<E> = SdkError<E, Response<SdkBody>>;
