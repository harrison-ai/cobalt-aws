//! # The harrison.ai AWS wrapper library
//!
//! This library provides a collection of wrappers around the
//! [aws-sdk-rust](https://github.com/awslabs/aws-sdk-rust) packages.
//!
//! These wrappers are intended to make it easier to perform common
//! tasks when developing applications which run on AWS infrastructure.
//!
//! * [Changelog](https://github.com/harrison-ai/hai-aws/CHANGELOG.md)
//!
//! ### About harrison.ai
//!
//! This crate is maintained by the Data Engineering team at [harrison.ai](https://harrison.ai).
//!
//! At [harrison.ai](https://harrison.ai) our mission is to create AI-as-a-medical-device solutions through
//! ventures and ultimately improve the standard of healthcare for 1 million lives every day.
//!

// Public modules

pub mod s3;

// Internal shared modules
mod localstack;
