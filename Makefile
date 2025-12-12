.PHONY: build-attestor-image

build-attestor-image:
	docker build -t attestor-local -f apps/ibc-attestor/Dockerfile .
