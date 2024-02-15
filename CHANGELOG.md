# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

## 0.13.1

 - aws-sdk-athena: Upgraded from 1.10.0 1.14.0
 - aws-config: Upgraded from 1.1.2 to 1.1.5
 - aws-sdk-s3: Upgraded from 1.12.0 to 1.15.0.
 - aws-sdk-sqs: Upgraded from 1.10.0 to 1.13.0.
 - aws-smithy-runtime-api: Upgraded from 1.1.2 to 1.1.5.
 - aws-smithy-async: Upgraded from 1.1.2. to 1.1.5
 - aws-types: Upgraded from 1.1.2 to 1.1.5
 - aws_lambda_events: Upgrade from 0.13 to 0.14.
 - lambda_runtime: Upgraded from 0.9.1 to 0.9.2
 - clap: Upgraded from 4.4 to 4.5
 - tokio: Upgraded from 1.35 to 1.36
 - Sorted the cargo dependencies.

## 0.13.0

 - aws-sdk-athena: Upgraded from 0.29 to 1.10.0.
 - aws-config: Updated to 1.1.2 with the behavior-version-latest feature.
 - aws-sdk-s3: Upgraded from 0.29 to 1.12.0.
 - aws-sdk-sqs: Upgraded from 0.29 to 1.10.0.
 - aws-types: Upgraded from 0.56 to 1.1.2.
 - aws-smithy-async: Added 1.1.2.
 - aws-smithy-runtime-api: Added 1.1.2.
 - aws-smithy-http: Removed.
 - Upgraded localstack to 3.0.2
 - Fixed integration tests

## 0.12.0

 - Upgrades from Docker Compose V1 to Docker Compose V2 for developer tooling.
 - Updates dependencies, excluding the AWS SDK, to their latest version. The AWS SDK update has a lot of breaking changes.
 - Fixes a bug with `make test-examples` where destroying the previous localstack container wasn't removing the named volumes. This resulted in subsequent re-runs to error, as the queue being created already existed.
 - Changes dependency versions to `major.minor` instead of `major.minor.build`, as the package is a library and we should let downstream packages select their desired build versions.

## 0.11.1

 - Fixed `run_local_handler` to support message handlers with return types other than `()`.

## 0.11.0

 - Added `running_on_lambda()` to support programs running both locally and on AWS Lambda.
 - Added `LocalContext` and `run_local_handler` to support executing message handlers locally.
 - **Breaking:** Updated `LambdaContext` to require an `EventType` type, which implements the `RunnableEventType` trait. To resolve this in your existing code, change the definition:

```rust
 impl LambdaContext<Env> for Context {
```
to

```rust
use cobalt_aws::lambda::SqsEvent;

impl LambdaContext<Env, SqsEvent> for Context {
```
 - **Breaking:** Updates AWS SDK dependencies to:
    - `aws-config = "0.55.3"`
    - `aws-sdk-athena = "0.28.0"`
    - `aws-sdk-s3 = "0.28.0"`
    - `aws-sdk-sqs = "0.28.0"`
    - `aws-smithy-http = "0.55.3"`
    - `aws-types = "0.55.3"`
    - `aws_lambda_events = "0.10.0"`
    - `lambda_runtime = "0.8.1"`

## 0.10.0

- Updated various AWS SDK dependencies. Note that the new versions include significant changes to endpoint URL resolution, which may introduce breaking changes for consumers; see the [`aws-sdk-rust` release notes](https://github.com/awslabs/aws-sdk-rust/releases/tag/release-2023-01-13) for details.
  - Updated `aws-config ` to 0.54.1
  - Updated `aws-sdk-athena` to 0.24.0
  - Updated `aws-sdk-s3` to 0.24.0
  - Updated `aws-sdk-sqs` to 0.24.0
  - Updated `aws-smithy-http` 0.54.4
  - Updated `aws-types` to 0.54.1
- Updated rust crate `tokio` to 1.26.0
- Updated rust crate `clap` to 4.1.8
- Updated rust crate `bytesize` to 1.2.0
- Updated rust crate `serial_test` to v1

## 0.9.2

- Add support to continue an existing AsyncMultipartUpload.

## 0.9.1

- Set AsyncMultipartUpload state back when there is no capacity.

## 0.9.0

 - Added AsyncMultipartUpload.
 - Add S3Object.
 - Updated `aws_lambda_events` to 0.7.2
 - Updated `aws_lambda_runtime` to 0.7.1
 - Updated `aws-config` to 0.51.0
 - Updated `aws-sdk-athena` to 0.21.0
 - Updated `aws-sdk-s3` to 0.21.0
 - Updated `aws-sdk-sqs` to 0.21.0
 - Updated `aws-smithy-http` to 0.51.0
 - Updated `aws-types` to 0.51.0

## 0.8.0

 - Updated `aws-sdk-{athena,s3,sqs}` dependencies to `0.19.0`.
 - Updated `aws-{config, smithy-http, types}` dependencies to `0.49.0`.
 - Updated `aws_lambda_events` dependency to `0.7.0`.
 - Updated `lambda_runtime` dependency to `0.6.1`.
 - Updated `clap` dependency to `4.0.4`.

## 0.7.0

 - Updated `aws-sdk-*` dependencies to `0.16.0`.
 - Updated `lambda_runtime` dependency to `0.6.0`.

## 0.6.0

 - Fixed incorrect environment variable parsing when running Image-based lambdas in LocalStack.
 - Updated `aws-sdk-*` dependencies to `0.15.0`.

## 0.5.0

 - Added `config::load_from_env()`, which creates an `SdkConfig` with LocalStack support.
 - Deprecated `s3::get_client()`, `sqs::get_client()`, and `athena::get_client()`.
 - Updated `aws-sdk-*` dependencies to `0.13.0`.

## 0.4.0

 - Added support for processing messages concurrently by setting the `RECORD_CONCURRENCY` env var.
 - Updated `aws-sdk-*` dependencies to `0.9.0`.

## 0.3.0

 - Added `sqs::send_messages_concurrently()`, which sends a stream of messages to an SQS queue.
 - Added `sqs::get_client()`, which creates an SQS client with LocalStack support.
 - Reduced spurious task wake-ups when closing an `s3::AsyncPutObject`.
 - Updated `aws-sdk-*` dependencies to `0.8.0`.

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
