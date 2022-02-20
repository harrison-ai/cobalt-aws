# Choose the example to run
export EXAMPLE=hello_lambda

# Setup the environment
export ENV="GREETING=hello"

# Specify the message to send to the lambda
export MESSAGE="{\"target\": \"world\"}"

export DEFAULT_REGION=ap-southeast-2

# Restart the localstack daemon
localstack stop
localstack start -d --no-banner
localstack wait

# Setup the queue
export QUEUE_NAME=test-queue
awslocal sqs create-queue --queue-name="$QUEUE_NAME" --attributes '{ "VisibilityTimeout": "240" }' > /dev/null
QUEUE_URL=$(awslocal sqs list-queues | jq -r '.QueueUrls[0]')
QUEUE_ARN=$(awslocal sqs get-queue-attributes --queue-url="$QUEUE_URL" --attribute-names QueueArn | jq -r '.Attributes.QueueArn')
export QUEUE_URL QUEUE_ARN

# Build the app and bundle it into a zip file
if [ -z "$CI" ]; then
   cargo b --example $EXAMPLE --target x86_64-unknown-linux-musl
   cp target/x86_64-unknown-linux-musl/debug/examples/$EXAMPLE bootstrap && zip lambda.zip bootstrap && rm bootstrap
else
   cargo b --example $EXAMPLE
   cp target/debug/examples/$EXAMPLE bootstrap && zip lambda.zip bootstrap && rm bootstrap
fi

# Create a lambda function
awslocal lambda create-function \
   --function-name=$EXAMPLE \
   --role=rn:aws:iam:local \
   --zip-file=fileb://lambda.zip \
   --environment "Variables={$ENV}" \
   --runtime=provided > /dev/null

# Create an event source mapping
awslocal lambda create-event-source-mapping --function-name $EXAMPLE --event-source-arn "$QUEUE_ARN" > /dev/null

# Send the message to the queue, triggering the lambda
awslocal sqs send-message --queue-url "$QUEUE_URL" --message-body "$MESSAGE" > /dev/null

echo
echo "🚀 Message $MESSAGE sent to '$EXAMPLE'."
echo "Run ./scripts/read_logs.sh to see the logs"
