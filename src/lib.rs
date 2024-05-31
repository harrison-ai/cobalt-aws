//! # The Cobalt AWS wrapper library
//!
//! This library provides a collection of wrappers around the
//! [aws-sdk-rust](https://github.com/awslabs/aws-sdk-rust) and
//! [lambda_runtime](https://github.com/awslabs/aws-lambda-rust-runtime) packages.
//!
//! These wrappers are intended to make it easier to perform common
//! tasks when developing applications which run on AWS infrastructure.
//!
//! * [Changelog](https://github.com/harrison-ai/cobalt-aws/blob/main/CHANGELOG.md)
//!
//! ### About harrison.ai
//!
//! This crate is maintained by the Data Engineering team at [harrison.ai](https://harrison.ai).
//!
//! At [harrison.ai](https://harrison.ai) our mission is to create AI-as-a-medical-device solutions through
//! ventures and ultimately improve the standard of healthcare for 1 million lives every day.
//!

// Public modules

#[cfg(feature = "athena")]
pub mod athena;
pub mod config;
#[cfg(feature = "lambda")]
pub mod lambda;
#[cfg(feature = "s3")]
pub mod s3;
#[cfg(feature = "sqs")]
pub mod sqs;
#[cfg(feature = "s3")]
pub mod types;
// Internal shared modules
mod localstack;
