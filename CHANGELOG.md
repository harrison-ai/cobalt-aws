# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

## 0.2.0

 - Added `athena::get_client()`, which creates an Athena client with LocalStack support.
 - Added `s3::get_object()`, which retrieves an object from S3 as an AsyncBufRead.
 - Added `s3::list_objects()`, which performs a bucket listing, returning a stream of results.
 - Added `s3::AsyncPutObject`, which implements the AsyncWrite trait and writes data to S3 using the put_object API.
 - Added `lambda::run_message_handler()`, which simplifies the task of writing a Lambda function which is triggered by an SQS event source mapping.
 - Moved test dependencies into `dev-dependencies`.
 - Fixed broken link in docs.
 - Updated `aws-sdk-*` dependencies to `0.7.0`.
 - Updated `aws_lambda_events` dependency to `0.6.0`.

## 0.1.0

- Initial release of the `cobalt-aws` crate.
