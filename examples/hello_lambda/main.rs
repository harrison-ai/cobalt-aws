use anyhow::Result;
use async_trait::async_trait;
use clap::Parser;
use serde::Deserialize;
use std::fmt::Debug;
use std::sync::Arc;

use cobalt_aws::lambda::{
    run_local_handler, run_message_handler, running_on_lambda, Error, LambdaContext, LocalContext,
    SqsEvent,
};

#[tokio::main]
async fn main() -> Result<(), Error> {
    if running_on_lambda()? {
        run_message_handler(message_handler).await
    } else {
        run_local_handler(message_handler).await
    }
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
impl LambdaContext<Env, SqsEvent> for Context {
    /// Initialise a shared context object from which will be
    /// passed to all instances of the message handler.
    async fn from_env(env: &Env) -> Result<Context> {
        Ok(Context {
            greeting: env.greeting.clone(),
        })
    }
}

#[async_trait]
impl LocalContext<Message> for Context {
    /// Initialise a shared context object from which will be
    /// passed to all instances of the message handler.
    async fn from_local() -> Result<Self> {
        Ok(Context {
            greeting: "Hello".to_string(),
        })
    }

    /// Construct a message object to be processed by the message handler.
    async fn msg(&self) -> Result<Message> {
        Ok(Message {
            target: "World".to_string(),
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
