.PHONY: build test clean

export GOCACHE := /tmp/expos-go-cache

build:
	mkdir -p bin
	go build -o bin/ayo ./cmd/ayo

test:
	go test ./...

clean:
	rm -rf bin
