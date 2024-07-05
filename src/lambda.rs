//! A wrapper around the [lambda_runtime](https://github.com/awslabs/aws-lambda-rust-runtime) crate.
//!
//! This wrapper provides wrappers to make it easier to write Lambda functions which consume messages from
//! various events, such as an SQS queue configured with an [event source mapping](https://docs.aws.amazon.com/lambda/latest/dg/invocation-eventsourcemapping.html),
//! or a [step function](https://docs.aws.amazon.com/step-functions/latest/dg/welcome.html).
//! It also provides mechanisms for running message handlers locally, without the full AWS Lambda environment.

use anyhow::{Context as _, Result};
use async_trait::async_trait;
/// Re-export from [aws_lambda_events::event::sqs::SqsEvent], with [RunnableEventType] implemented.
pub use aws_lambda_events::event::sqs::SqsEvent;
use clap::Parser;
use futures::stream::{self, StreamExt, TryStreamExt};
use futures::FutureExt;
use lambda_runtime::{service_fn, LambdaEvent};
use serde::Serialize;
use std::ffi::OsString;
use std::future::Future;
use std::iter::empty;
use std::sync::Arc;
use tracing_subscriber::filter::EnvFilter;

/// Re-export of [lambda_runtime::Error](https://docs.rs/lambda_runtime/latest/lambda_runtime/type.Error.html).
///
// We provide this re-export so that the user doesn't need to have lambda_runtime as a direct dependency.
pub use lambda_runtime::Error;

/// This struct is used to attempt to parse the `AWS_LAMBDA_FUNCTION_NAME` environment variable.
///
/// We assume that if this variable is present then we're running in a Lambda function.
/// https://docs.aws.amazon.com/lambda/latest/dg/configuration-envvars.html
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

/// A trait of the `Context` type, required when using [run_message_handler] to execute the message handler.
///
/// All `Context` types must implement the implement the [LambdaContext::from_env] method for their corresponding `Env` type
/// in order to use the `Context` with [run_message_handler].
///
/// When implementing `LambdaContext` you must also specify the `EventType` that the Lambda expects to receive, e.g. [SqsEvent].
#[async_trait]
pub trait LambdaContext<Env, EventType>: Sized {
    /// # Example
    ///
    /// ```no_run
    /// use anyhow::Result;
    /// use cobalt_aws::lambda::{LambdaContext, SqsEvent};
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
    /// impl LambdaContext<Env, SqsEvent> for Context {
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

/// A trait that allows a type to be used as the `EventType` trait of a [LambdaContext].
#[async_trait(?Send)]
pub trait RunnableEventType<Msg, MsgResult, EventResult> {
    async fn process<F, Fut, Context>(
        &self,
        message_handler: F,
        ctx: Arc<Context>,
    ) -> Result<EventResult>
    where
        F: Fn(Msg, Arc<Context>) -> Fut,
        Fut: Future<Output = Result<MsgResult>>,
        Msg: serde::de::DeserializeOwned;
}

/// The `StepFunctionEvent` trait allows any deserializable struct
/// to be used as the EventType for a `LambdaContext`.
///
/// # Example
///
/// ``` no_run
/// use async_trait::async_trait;
/// use cobalt_aws::lambda::StepFunctionEvent;
/// use serde::{Deserialize, Serialize};
///
/// /// The structure of the event we expect to see at the Step Function.
/// #[derive(Debug, Deserialize, Serialize, Clone)]
/// pub struct MyEvent {
///     pub greeting: String,
/// }
///
/// impl StepFunctionEvent for MyEvent {}
///
/// #[async_trait]
/// impl LambdaContext<Env, StatsEvent> for Context {
///     ...
/// }
/// ```
pub trait StepFunctionEvent: Clone {}

#[async_trait(?Send)]
impl<T: StepFunctionEvent, MsgResult> RunnableEventType<T, MsgResult, MsgResult> for T {
    async fn process<F, Fut, Context>(
        &self,
        message_handler: F,
        ctx: Arc<Context>,
    ) -> Result<MsgResult>
    where
        F: Fn(T, Arc<Context>) -> Fut,
        Fut: Future<Output = Result<MsgResult>>,
    {
        message_handler(self.clone(), ctx).await
    }
}

// When processing an SqsEvent, we expect the message handler to return (), and the
// lambda itself to also return ().
#[async_trait(?Send)]
impl<Msg> RunnableEventType<Msg, (), ()> for SqsEvent {
    async fn process<F, Fut, Context>(&self, message_handler: F, ctx: Arc<Context>) -> Result<()>
    where
        F: Fn(Msg, Arc<Context>) -> Fut,
        Fut: Future<Output = Result<()>>,
        Msg: serde::de::DeserializeOwned,
    {
        // Process the records in the event batch concurrently, up to `RECORD_CONCURRENCY`.
        // If any of them fail, return immediately.
        let handler_env: HandlerEnv = HandlerEnv::try_parse_from(empty::<OsString>())
            .context("An error occurred while parsing environment variable for handler")?;
        stream::iter(&self.records)
            .map(|record| {
                let body = record.body.clone();
                let body = body.with_context(|| "No SqsMessage body".to_string())?;
                let msg = serde_json::from_str::<Msg>(&body)
                    .with_context(|| format!("Error parsing body into message: {}", body))?;
                Ok((msg, body))
            })
            .try_for_each_concurrent(handler_env.record_concurrency, |(msg, body)| async {
                message_handler(msg, ctx.clone())
                    .map(move |r| {
                        r.with_context(|| format!("Error running message handler {}", body))
                    })
                    .await?;
                Ok(())
            })
            .await
    }
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
/// use cobalt_aws::lambda::{run_message_handler, Error, LambdaContext, SqsEvent};
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
/// impl LambdaContext<Env, SqsEvent> for Context {
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
pub async fn run_message_handler<EventType, EventOutput, F, Fut, R, Msg, Context, Env>(
    message_handler: F,
) -> Result<(), Error>
where
    EventType: for<'de> serde::de::Deserialize<'de> + RunnableEventType<Msg, R, EventOutput>,
    F: Fn(Msg, Arc<Context>) -> Fut,
    Fut: Future<Output = Result<R>>,
    Msg: serde::de::DeserializeOwned,
    Context: LambdaContext<Env, EventType> + std::fmt::Debug,
    EventOutput: Serialize,
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
    let init_result = async {
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
    }
    .await;

    lambda_runtime::run(service_fn(|event: LambdaEvent<EventType>| async {
        // Check the result of the init phase. If it failed, log the message
        // and immediately return.
        let ctx = match &init_result {
            Ok(x) => x,
            Err(e) => {
                tracing::error!("{:?}", e);
                return Err(Error::from("Failed to initialise lambda."));
            }
        };

        // Process the event and capture any errors
        let (event, _context) = event.into_parts();
        let result = event.process(&message_handler, ctx.clone()).await;

        // Log out the full error, as the lambda_runtime only logs the first line of the error
        // message, which can hide crucial information.
        match result {
            Ok(x) => Ok(x),
            Err(e) => {
                tracing::error!("{:?}", e);
                Err(Error::from("Failed to process SQS event."))
            }
        }
    }))
    .await
}

/// A trait of the `Context` type, required when using [run_local_handler] to execute the message handler.
///
/// All `Context` types must implement the implement the [LocalContext::from_local] and [LocalContext::msg] methods for their corresponding `Msg` type
/// in order to use the `Context` with [run_local_handler].
///
/// # Example
///
/// ```no_run
/// use anyhow::Result;
/// use serde::Deserialize;
/// use cobalt_aws::lambda::LocalContext;
///
/// # use async_trait::async_trait;
/// # #[derive(Debug)]
/// # pub struct Env {
/// #     pub greeting: String,
/// # }
/// #
/// /// Shared context we build up before invoking the local runner.
/// #[derive(Debug)]
/// pub struct Context {
///     pub greeting: String,
/// }
///
/// #[derive(Debug, Deserialize)]
/// pub struct Message {
///     pub target: String,
/// }
///
/// #[async_trait]
/// impl LocalContext<Message> for Context {
///     /// Initialise a shared context object from which will be
///     /// passed to all instances of the message handler.
///     async fn from_local() -> Result<Self> {
///         Ok(Context {
///             greeting: "Hello".to_string(),
///         })
///     }
///
///     /// Construct a message object to be processed by the message handler.
///     async fn msg(&self) -> Result<Message> {
///         Ok(Message {
///             target: "World".to_string(),
///         })
///     }
/// }
/// ```
#[async_trait]
pub trait LocalContext<Msg>: Sized {
    /// Construct a new local Context object.
    async fn from_local() -> Result<Self>;
    /// Construct a message object to be processed by the message handler.
    async fn msg(&self) -> Result<Msg>;
}

/// Executes a message handler against the message provided by the [LocalContext].
///
/// The `run_local_handler` function takes care of the following tasks:
///
/// * Sets up tracing to ensure all `tracing::<...>!()` calls are JSON formatted for consumption by CloudWatch.
/// * Initialises a shared context object, which is passed to your handler.
/// * Initialises a single message object, which is passed to your handler.
///
/// ## Writing a message handler
///
/// To write a message handler, you need to define four elements:
///
/// * The `Message` structure, which defines the structure of the messages which will be sent to your message handler.
/// * The `Context` structure, which represents the shared state
///   that will be passed into your message handler. This structure needs to implement the [LocalContext] trait.
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
/// use cobalt_aws::lambda::{run_local_handler, Error, LocalContext};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Error> {
///     let result = run_local_handler(message_handler).await?;
///     Ok(())
/// }
///
/// /// The structure of the messages to be processed by our message handler.
/// #[derive(Debug, Deserialize)]
/// pub struct Message {
///     pub target: String,
/// }
///
///
/// /// Shared context we build up before invoking the local runner.
/// #[derive(Debug)]
/// pub struct Context {
///     pub greeting: String,
/// }
///
/// #[async_trait]
/// impl LocalContext<Message> for Context {
///     /// Initialise a shared context object which will be
///     /// passed to the message handler.
///     async fn from_local() -> Result<Self> {
///         Ok(Context {
///             greeting: "Hello".to_string(),
///         })
///     }
///
///     /// Construct a message to be processed.
///     async fn msg(&self) -> Result<Message> {
///         Ok(Message {
///             target: "World".to_string(),
///         })
///     }
/// }
///
/// /// Process a single message, within the given context.
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
pub async fn run_local_handler<F, Fut, R, Msg, Context>(message_handler: F) -> Result<R, Error>
where
    F: Fn(Msg, Arc<Context>) -> Fut,
    Fut: Future<Output = Result<R>>,
    Msg: serde::de::DeserializeOwned,
    Context: LocalContext<Msg> + std::fmt::Debug,
{
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    let ctx = Arc::new(Context::from_local().await?);
    tracing::info!("Context: {:?}", ctx);
    Ok(message_handler(ctx.msg().await?, ctx).await?)
}
