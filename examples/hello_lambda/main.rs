use anyhow::Result;
use async_trait::async_trait;
use clap::Parser;
use serde::Deserialize;
use std::fmt::Debug;
use std::sync::Arc;

use cobalt_aws::lambda::{run_message_handler, Error, LambdaContext};

#[tokio::main]
async fn main() -> Result<(), Error> {
    run_message_handler(message_handler).await
}

/// The structure of the messages we expect to see on the queue.
#[derive(Debug, Deserialize)]
pub struct Message {
    pub target: String,
}

/// Configuration we receive from environment variables.
///
/// Note: all fields should be tagged with the `#[arg(env)]` attribute.
#[derive(Debug, Parser)]
pub struct Env {
    #[arg(env)]
    pub greeting: String,
}

/// Shared context we build up before invoking the lambda runner.
#[derive(Debug)]
pub struct Context {
    pub greeting: String,
}

#[async_trait]
impl LambdaContext<Env> for Context {
    /// Initialise a shared context object from which will be
    /// passed to all instances of the message handler.
    async fn from_env(env: &Env) -> Result<Context> {
        Ok(Context {
            greeting: env.greeting.clone(),
        })
    }
}

/// Process a single message from the SQS queue, within the given context.
async fn message_handler(message: Message, context: Arc<Context>) -> Result<()> {
    tracing::debug!("Message: {:?}", message);
    tracing::debug!("Context: {:?}", context);

    // Log a greeting to the target
    tracing::info!("{}, {}!", context.greeting, message.target);

    Ok(())
}
