
UID = $(shell id -u)
DCRUN = docker-compose run --rm --user $(UID)

.DEFAULT_GOAL := help

## test:         run all the tests
test:
	$(DCRUN) cargo test

## test-examples:	test example code using localstack
# FIXME: turn this into an actual test with assertions
# Currently you need to visually inspect that the correct log output is generated.
test-examples:
	./scripts/run_example.sh
	sleep 20
	./scripts/read_logs.sh

## fmt:          format all code using standard conventions
fmt:
	$(DCRUN) cargo fmt

## fix:          apply automated code style fixes
fix: fmt
	$(DCRUN) cargo fix --all-targets --all-features
	$(DCRUN) cargo clippy --fix --all-targets --all-features --no-deps

## validate:     perform style and consistency validation checks
validate:
	$(DCRUN) cargo fmt -- --check
	$(DCRUN) cargo clippy --all-targets --all-features --no-deps -- -D warnings
	$(DCRUN) cargo deny check

## pull:         docker-compose pull
pull:
	docker-compose pull

## help:         show this help
help:
	@sed -ne '/@sed/!s/## //p' $(MAKEFILE_LIST)


# Build all the docker-compose images
dc-build: .build.docker

.build.docker: docker-compose.yml $(shell find scripts -name 'Docker*')
	docker-compose build cargo awslocal
	touch $@
