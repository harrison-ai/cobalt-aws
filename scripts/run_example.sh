set -e

# Choose the example to run
export EXAMPLE=hello_lambda

# Setup the environment
export ENV="GREETING=hello"

# Specify the message to send to the lambda
export MESSAGE="{\"target\": \"world\"}"

DCRUN="docker compose run --rm"
AWSLOCAL="$DCRUN awslocal"

# Restart the localstack daemon, to clear state.
docker compose down --volumes
docker compose up -d localstack

# TODO: cleanly wait for localstack to come up
sleep 5

# Setup the queue
export QUEUE_NAME=test-queue
$AWSLOCAL sqs create-queue --queue-name="$QUEUE_NAME" --attributes '{ "VisibilityTimeout": "240" }'
QUEUE_URL=$($AWSLOCAL sqs list-queues | jq -r '.QueueUrls[0]')
QUEUE_ARN=$($AWSLOCAL sqs get-queue-attributes --queue-url="$QUEUE_URL" --attribute-names QueueArn | jq -r '.Attributes.QueueArn')
export QUEUE_URL QUEUE_ARN

# Build the app and bundle it into a zip file.
# Need to strip debuginfo to keep it under maximum lambda file size.
$DCRUN -e RUSTFLAGS="-C strip=debuginfo" cargo build --example $EXAMPLE --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/debug/examples/$EXAMPLE bootstrap && zip lambda.zip bootstrap && rm bootstrap

# Create a lambda function
$AWSLOCAL lambda create-function \
   --function-name=$EXAMPLE \
   --role=rn:aws:iam:local \
   --zip-file=fileb://lambda.zip \
   --environment "Variables={$ENV}" \
   --runtime=provided

# Create an event source mapping
$AWSLOCAL lambda create-event-source-mapping --function-name $EXAMPLE --event-source-arn "$QUEUE_ARN"

# Send the message to the queue, triggering the lambda
$AWSLOCAL sqs send-message --queue-url "$QUEUE_URL" --message-body "$MESSAGE"

echo
echo "🚀 Message $MESSAGE sent to '$EXAMPLE'."
echo "Run ./scripts/read_logs.sh to see the logs"
