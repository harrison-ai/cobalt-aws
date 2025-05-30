set -ex

DCRUN="docker compose run --rm"
AWSLOCAL="$DCRUN awslocal"

export EXAMPLE=hello_lambda
export DEFAULT_REGION=ap-southeast-2
export EXPECTED_MESSAGE="hello, world!"

# Read the logs
export GROUP_NAME="/aws/lambda/${EXAMPLE}"

sleep 60

for container in $(docker ps --filter "name=localstack-main-lambda" --format "{{.Names}}"); do
  echo "=== Logs for container: $(docker inspect --format='{{.Name}}' $container) ==="
  docker logs $container
  message=$(docker logs $container | grep "$EXPECTED_MESSAGE")
  if [ -z "$message" ] || [ "$message" = "null" ]; then
    echo "Test failed: Expected message not found in output"
    exit 1
  else
    echo "Test passed! Found expected message: $message"
    exit 0
  fi
  echo
done

echo "Test failed: Lambda container output not found"
exit 1
