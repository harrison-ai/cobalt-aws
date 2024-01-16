
UID = $(shell id -u)
DCRUN = docker compose run --rm --user $(UID)

.DEFAULT_GOAL := help

## test:         run all the tests
test:
	$(DCRUN) cargo test

## test-examples:	test example code using localstack
# Sleep for 20s in between to let CI complete execution
test-examples:
	./scripts/run_example.sh
	sleep 1
	./scripts/read_and_validate_logs.sh

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

## licenses-report: Build license summary file
licenses-report:
	rm Cargo.lock || 1
	$(DCRUN) cargo about generate --output-file ./licenses/licenses.html about.hbs

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
