#!/usr/bin/env bash
printf "Configuring localstack components..."

set -x

# Any initialisation of the localstack infrastructure required for testing
# should be set up in this script.

## S3 tests
awslocal s3 mb s3://empty-bucket

# Add enough data to require multiple pages of data when listing the bucket
mkdir -p /tmp/test-data/multi-page
set +x
for i in `seq -w 1 2500`
do
  cp /tmp/test-data/test.txt /tmp/test-data/multi-page/file-${i}.txt
done
set -x

awslocal s3 mb s3://test-bucket
awslocal s3 cp /tmp/test-data s3://test-bucket --recursive

printf "Configuration done"
