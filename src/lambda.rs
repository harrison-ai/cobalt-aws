//! A wrapper around the [lambda_runtime](https://github.com/awslabs/aws-lambda-rust-runtime) crate.
//!
//! This wrapper provides wrappers to make it easier to write Lambda functions which consume message from
//! an SQS queue configured with an [event source mapping](https://docs.aws.amazon.com/lambda/latest/dg/invocation-eventsourcemapping.html).

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use aws_lambda_events::event::sqs::SqsEvent;
use clap::Parser;
use futures::stream::{self, StreamExt, TryStreamExt};
use futures::FutureExt;
use lambda_runtime::{service_fn, LambdaEvent};
use std::ffi::OsString;
use std::future::Future;
use std::iter::empty;
use std::sync::Arc;
use tracing_subscriber::filter::EnvFilter;

/// Re-export of [lambda_runtime::Error](https://docs.rs/lambda_runtime/latest/lambda_runtime/type.Error.html).
///
// We provide this re-export so that the user doesn't need to have lambda_runtime as a direct dependency.
pub use lambda_runtime::Error;

#[derive(Debug, Parser)]
struct CheckLambda {
    #[arg(env)]
    aws_lambda_function_name: Option<String>,
}

/// Determine whether the code is being executed within an AWS Lambda.
///
/// This function can be used to write binaries that are able to run both locally
/// or as a Lambda function.
pub fn running_on_lambda() -> Result<bool> {
    let check_lambda = CheckLambda::try_parse_from(empty::<OsString>())
        .context("An error occurred while parsing environment variables for message context.")?;
    Ok(check_lambda.aws_lambda_function_name.is_some())
}


/// A required trait of the `Context` type used by message handler functions in [run_message_handler].
///
/// All `Context` types must implement the implement the [LambdaContext::from_env] method for their corresponding `Env` type.
#[async_trait]
pub trait LambdaContext<Env>: Sized {
    /// # Example
    ///
    /// ```no_run
    /// use anyhow::Result;
    /// use cobalt_aws::lambda::LambdaContext;
    ///
    /// # use async_trait::async_trait;
    /// # #[derive(Debug)]
    /// # pub struct Env {
    /// #     pub greeting: String,
    /// # }
    /// #
    /// /// Shared context we build up before invoking the lambda runner.
    /// #[derive(Debug)]
    /// pub struct Context {
    ///     pub greeting: String,
    /// }
    ///
    /// #[async_trait]
    /// impl LambdaContext<Env> for Context {
    ///     /// Initialise a shared context object from which will be
    ///     /// passed to all instances of the message handler.
    ///     async fn from_env(env: &Env) -> Result<Context> {
    ///         Ok(Context {
    ///             greeting: env.greeting.clone(),
    ///         })
    ///     }
    /// }
    /// ```
    async fn from_env(env: &Env) -> Result<Self>;
}

/// Environment variables to configure the lambda handler.
#[derive(Debug, Parser)]
struct HandlerEnv {
    /// How many concurrent records should be processed at once.
    /// This defaults to 1 to set synchronous processing.
    /// This value should not exceed the value of `BatchSize`
    /// configured for the [event source mapping](https://docs.aws.amazon.com/lambda/latest/dg/invocation-eventsourcemapping.html#invocation-eventsourcemapping-batching).
    #[arg(env, default_value_t = 1)]
    record_concurrency: usize,
}

/// Executes a message handler against all the messages received in a batch
/// from an SQS event source mapping.
///
/// The `run_message_handler` function takes care of the following tasks:
///
/// * Executes the Lambda runtime, using [lambda_runtime](https://github.com/awslabs/aws-lambda-rust-runtime).
/// * Sets up tracing to ensure all `tracing::<...>!()` calls are JSON formatted for consumption by CloudWatch.
/// * Processes environment variables and makes them available to your handler
/// * Initialises a shared context object, which is passed to your handler.
/// * Deserialises a batch of messages and passes each one to your handler.
/// * Processes messages concurrently, based on the env var `RECORD_CONCURRENCY` (default: 1)
///
/// ## Writing a message handler
///
/// To write a message handler, you need to define four elements:
///
/// * The `Message` structure, which defines the structure of the messages which will be sent to the SQS
///    queue, and then forwarded on to your Lambda.
/// * The `Env` structure, which defines the expected environment variables your Lambda will receive.
/// * The `Context` structure, which is provided the `Env` structure, and represents the shared state
///   that will be passed into your message handler. This structure needs to implement the [LambdaContext] trait.
/// * The `message_handler` function, which accepts a `Message` and a `Context`, and performs the desired actions.
///
/// # Example
///
/// ```no_run
/// use anyhow::Result;
/// use async_trait::async_trait;
/// use clap::Parser;
/// use serde::Deserialize;
/// use std::fmt::Debug;
/// use std::sync::Arc;
///
/// use cobalt_aws::lambda::{run_message_handler, Error, LambdaContext};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Error> {
///     run_message_handler(message_handler).await
/// }
///
/// /// The structure of the messages we expect to see on the queue.
/// #[derive(Debug, Deserialize)]
/// pub struct Message {
///     pub target: String,
/// }
///
/// /// Configuration we receive from environment variables.
/// ///
/// /// Note: all fields should be tagged with the `#[arg(env)]` attribute.
/// #[derive(Debug, Parser)]
/// pub struct Env {
///     #[arg(env)]
///     pub greeting: String,
/// }
///
/// /// Shared context we build up before invoking the lambda runner.
/// #[derive(Debug)]
/// pub struct Context {
///     pub greeting: String,
/// }
///
/// #[async_trait]
/// impl LambdaContext<Env> for Context {
///     /// Initialise a shared context object from which will be
///     /// passed to all instances of the message handler.
///     async fn from_env(env: &Env) -> Result<Context> {
///         Ok(Context {
///             greeting: env.greeting.clone(),
///         })
///     }
/// }
///
/// /// Process a single message from the SQS queue, within the given context.
/// async fn message_handler(message: Message, context: Arc<Context>) -> Result<()> {
///     tracing::debug!("Message: {:?}", message);
///     tracing::debug!("Context: {:?}", context);
///
///     // Log a greeting to the target
///     tracing::info!("{}, {}!", context.greeting, message.target);
///
///     Ok(())
/// }
/// ```
/// # Concurrent processing
///
/// By default `run_message_handler` will process the messages in an event batch sequentially.
/// You can configure `run_message_handler` to process messages concurrently by setting
/// the `RECORD_CONCURRENCY` env var (default: 1). This value should not exceed the value of `BatchSize`
/// configured for the [event source mapping](https://docs.aws.amazon.com/lambda/latest/dg/invocation-eventsourcemapping.html#invocation-eventsourcemapping-batching)
/// (default: 10).
///
/// # Error handling
///
/// If any errors are raised during init, or from the `message_handler` function, then the entire message
/// batch will be considered to have failed. Error messages will be logged to stdout in a format compatible
/// with CloudWatch, and the message batch being processed will be returned to the original queue.
pub async fn run_message_handler<F, Fut, Msg, Context, Env>(message_handler: F) -> Result<(), Error>
where
    F: Fn(Msg, Arc<Context>) -> Fut,
    Fut: Future<Output = Result<()>>,
    Msg: serde::de::DeserializeOwned,
    Context: LambdaContext<Env> + std::fmt::Debug,
    Env: Parser + std::fmt::Debug,
{
    // Perform initial setup outside of the runtime to avoid this code being run
    // on every invocation of the lambda.
    //
    // Ideally an error in this code would cause the runtime to return an initialization error:
    // https://docs.aws.amazon.com/lambda/latest/dg/runtimes-api.html#runtimes-api-initerror
    // however this isn't currently supported by `lambda_runtime` (perhaps an area
    // for future work). To work around this, we capture any errors during this phase
    // and pass them into the handler function itself, so that it can raise the error
    // when the function is invoked.
    let init_result = (|| async {
        // Setup tracing
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .json()
            .init();
        // We only care about values provided as environment variables, so we pass in an empty
        // iterator, rather than having clap parse the command line arguments. This avoids an
        // unfortunate issue in LocalStack where a command line arg of "handler.handler" is provided
        // as a command line argument to the process when using an Image based lambda. This triggers
        // a problem, as clap (wrongly, IMHO) tries to assign this command line option to the first
        //element of the Env struct, and then ignores any actual environment variable provided.
        let env = Env::try_parse_from(empty::<OsString>()).context(
            "An error occurred while parsing environment variables for message context.",
        )?;
        let ctx = Arc::new(Context::from_env(&env).await?);
        tracing::info!("Env: {:?}", env);
        tracing::info!("Context: {:?}", ctx);
        Ok::<_, anyhow::Error>(ctx)
    })()
    .await;

    let handler_env: HandlerEnv = HandlerEnv::try_parse_from(empty::<OsString>())
        .context("An error occured while parsing environment variable for handler")?;

    lambda_runtime::run(service_fn(|event: LambdaEvent<SqsEvent>| async {
        let (event, _context) = event.into_parts();
        // Process the event and capture any errors
        let result = (|| async {
            // Check the result of the init phase. If it failed, log the message
            // and immediately return.
            let ctx = match &init_result {
                Ok(x) => x,
                Err(e) => {
                    tracing::error!("{:?}", e);
                    return Err(anyhow::anyhow!("Failed to initialise lambda."));
                }
            };

            // Process the records in the event batch concurrently, up to `RECORD_CONCURRENCY`.
            // If any of them fail, return immediately.
            stream::iter(event.records)
                .map(|record| {
                    let body = record
                        .body
                        .as_ref()
                        .with_context(|| format!("No SqsMessage body: {:?}", record))?;
                    let msg = serde_json::from_str::<Msg>(body)
                        .with_context(|| format!("Error parsing body into message: {}", body))?;
                    Ok((msg, body.to_owned()))
                })
                .try_for_each_concurrent(handler_env.record_concurrency, |(msg, body)| {
                    message_handler(msg, ctx.clone()).map(move |r| {
                        r.with_context(|| format!("Error running message handler {}", body))
                    })
                })
                .await
        })()
        .await;

        // Log out the full error, as the lambda_runtime only logs the first line of the error
        // message, which can hide crucial information.
        match result {
            Ok(_) => result,
            Err(e) => {
                tracing::error!("{:?}", e);
                Err(anyhow::anyhow!("Failed to process SQS event."))
            }
        }
    }))
    .await
}
