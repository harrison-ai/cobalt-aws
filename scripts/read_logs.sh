set -e

DCRUN="docker-compose run --rm"
AWSLOCAL="$DCRUN awslocal"

export EXAMPLE=hello_lambda
export DEFAULT_REGION=ap-southeast-2

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

$AWSLOCAL logs get-log-events --log-group-name /aws/lambda/$EXAMPLE --log-stream-name "$STREAM_NAME" --start-from-head | jq -r '.events[].message'
