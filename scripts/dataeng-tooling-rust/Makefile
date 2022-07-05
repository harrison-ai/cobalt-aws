
.DEFAULT_GOAL := help

## publish:			build docker images and push to registry
publish:
	./scripts/publish.sh

## help:			show this help
help:
	@sed -ne '/@sed/!s/## //p' $(MAKEFILE_LIST)
