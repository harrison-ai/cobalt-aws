set -e

DCRUN="docker-compose run --rm"
AWSLOCAL="$DCRUN awslocal"

export EXAMPLE=hello_lambda
export DEFAULT_REGION=ap-southeast-2
export EXPECTED_MESSAGE="hello, world!"

# Read the logs

export GROUP_NAME=`$AWSLOCAL logs describe-log-groups | jq -r '.logGroups[0].logGroupName'`
if [ -z "$GROUP_NAME" ] || [ "$GROUP_NAME" = "null" ]; then
    echo "Failed to determine group name"
    exit 1
fi

export STREAM_NAME=`$AWSLOCAL logs describe-log-streams --log-group-name $GROUP_NAME | jq -r '.logStreams[].logStreamName'`
if [ -z "$STREAM_NAME" ] || [ "$STREAM_NAME" = "null" ]; then
    echo "Failed to determine stream name"
    exit 1
fi

export LOGS=$($AWSLOCAL logs get-log-events --log-group-name /aws/lambda/$EXAMPLE --log-stream-name "$STREAM_NAME" --start-from-head \
 | jq -r '.events[].message')

log=$(echo $LOGS | sed -e 's/.*LATEST\(.*\)END.*/\1/' |  sed 's/\x1B\[[0-9;]\{1,\}[A-Za-z]//g')
if [ -z "$log" ] || [ "$log" = "null" ]; then
  echo "Empty logs"
  exit 1
fi

message=$(echo $message | jq -r ".fields.message")
if [ "$message" = "$EXPECTED_MESSAGE" ]; then
  echo "Test passed!"
  exit 0
else
  echo "Test failed: EXPECTED=$EXPECTED_MESSAGE; ACTUAL=$message."
  exit 1
fi