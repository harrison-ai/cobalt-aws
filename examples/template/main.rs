//! A minimal template to use as a starting point when writing your lambda function.
use anyhow::Result;
use async_trait::async_trait;
use clap::Parser;
use cobalt_aws::lambda::{
    run_local_handler, run_message_handler, running_on_lambda, Error, LambdaContext, LocalContext,
};
use serde::Deserialize;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Error> {
    if running_on_lambda()? {
        run_message_handler(message_handler).await
    } else {
        run_local_handler(message_handler).await
    }
}

#[derive(Debug, Deserialize)]
pub struct Message {}

#[derive(Debug, Parser)]
pub struct Env {}

#[derive(Debug)]
pub struct Context {}

#[async_trait]
impl LambdaContext<Env> for Context {
    async fn from_env(env: &Env) -> Result<Context> {
        tracing::info!("Env: {:?}", env);

        Ok(Context {})
    }
}

#[async_trait]
impl LocalContext<Message> for Context {
    async fn from_local() -> Result<Self> {
        Ok(Context {})
    }

    async fn msg(&self) -> Result<Message> {
        Ok(Message {})
    }
}

async fn message_handler(message: Message, context: Arc<Context>) -> Result<()> {
    tracing::info!("Message: {:?}", message);
    tracing::info!("Context: {:?}", context);

    Ok(())
}
