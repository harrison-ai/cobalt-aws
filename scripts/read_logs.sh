export EXAMPLE=hello_lambda
export DEFAULT_REGION=ap-southeast-2

# Read the logs

export GROUP_NAME=`awslocal logs describe-log-groups | jq -r '.logGroups[0].logGroupName'`
export STREAM_NAME=`awslocal logs describe-log-streams --log-group-name $GROUP_NAME | jq -r '.logStreams[0].logStreamName'`
awslocal logs get-log-events --log-group-name /aws/lambda/$EXAMPLE --log-stream-name "$STREAM_NAME" | jq
