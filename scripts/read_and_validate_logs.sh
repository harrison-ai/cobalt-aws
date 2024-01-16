set -ex

DCRUN="docker compose run --rm"
AWSLOCAL="$DCRUN awslocal"

export EXAMPLE=hello_lambda
export DEFAULT_REGION=ap-southeast-2
export EXPECTED_MESSAGE="hello, world!"

# Read the logs
export GROUP_NAME="/aws/lambda/${EXAMPLE}"

if ! timeout 60 bash -c '
count=0
while true; do 
    if '"${AWSLOCAL}"' logs describe-log-groups --log-group-name-prefix "'${GROUP_NAME}'" | grep -q "'${GROUP_NAME}'"; then
        exit 0
    fi
    sleep 1
    ((count++))
    if ((count % 1 == 0)); then
        echo "Still waiting for log group '"${GROUP_NAME}"' to appear..."
    fi
done
'; then
    echo "Error: Timed out waiting for log group ${GROUP_NAME} to appear."
fi

export STREAM_NAME=`$AWSLOCAL logs describe-log-streams --log-group-name $GROUP_NAME | jq -r '.logStreams[].logStreamName'`
if [ -z "$STREAM_NAME" ] || [ "$STREAM_NAME" = "null" ]; then
    echo "Failed to determine stream name"
    exit 1
fi

AWS_LOGS="$($AWSLOCAL logs get-log-events --log-group-name /aws/lambda/$EXAMPLE --log-stream-name "$STREAM_NAME" --start-from-head | jq -r '.events[].message')"

# Extracting the message and removing ASCII color tags
log=$(echo $AWS_LOGS | sed -e 's/.*LATEST\(.*\)END.*/\1/' |  sed 's/\x1B\[[0-9;]\{1,\}[A-Za-z]//g')
if [ -z "$log" ] || [ "$log" = "null" ]; then
  echo "Empty logs"
  exit 1
fi
message=$(echo $log | jq -r ".fields.message")
if [ "$message" = "$EXPECTED_MESSAGE" ]; then
  echo "Test passed! Found expected message: $message"
  exit 0
else
  echo "Test failed: EXPECTED=$EXPECTED_MESSAGE; ACTUAL=$message."
  exit 1
fi
