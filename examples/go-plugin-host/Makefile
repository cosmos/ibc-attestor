BIN := bin

.PHONY: setup proto build keystore run clean

# One-time setup: build both binaries, generate stubs, create a throwaway keystore.
setup: build keystore

# Build the attestor and host into ./bin so the host finds the attestor beside it.
build: proto
	cd ../.. && cargo build -p ibc-attestor --bin ibc_attestor
	mkdir -p $(BIN)
	cp ../../target/debug/ibc_attestor $(BIN)/ibc_attestor
	go build -o $(BIN)/host .

proto:
	./generate.sh

keystore: build
	mkdir -p keystore
	@if [ ! -f keystore/ibc-attestor-keystore ]; then \
		$(BIN)/ibc_attestor key generate --keystore ./keystore --keystore-password poc; \
	else \
		echo "keystore already exists, skipping"; \
	fi

run: build
	./$(BIN)/host

clean:
	rm -rf keystore gen bin
