
UID = $(shell id -u)
DCRUN = docker-compose run --rm --user $(UID)

.DEFAULT_GOAL := help

## test:         run all the tests
test:
	$(DCRUN) cargo test

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
