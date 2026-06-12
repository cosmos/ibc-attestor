# Remote signing

For production deployments, the attestor delegates signing to an external gRPC service rather than holding key material on the host. This keeps secp256k1 private keys off the attestor process — typically in an HSM, KMS, or dedicated signing daemon — and lets the operator rotate keys without restarting the attestor.

The remote signer is selected at startup with `--signer-type remote`.

## Configuration

Settings live in the `[signer]` table of the attestor config (`--config <path>`). See [`docs/configuration.md`](configuration.md) for the field reference; the minimum is:

```toml
[signer]
endpoint  = "https://remote-signer.example:50051"
wallet_id = "ibc-attestor-prod"
# service_account_token_path = "/var/run/secrets/kubernetes.io/serviceaccount/token"   # optional
```

| Field                         | Required | Description |
|-------------------------------|----------|-------------|
| `endpoint`                    | yes      | gRPC URL of the remote signer service. Must use `https://` unless `allow_insecure_plaintext` is explicitly enabled for tests. |
| `wallet_id`                   | yes      | Identifier of the secp256k1 wallet on the signing service whose address is registered with the on-chain light client. Sent in every `SignRequest`. |
| `allow_insecure_plaintext`    | no       | Allows `http://` plaintext gRPC. Defaults to `false`; intended only for non-production test environments. |
| `service_account_token_path`  | no       | Path to a file containing a bare JWT (no JSON envelope) — the format Kubernetes populates `kubernetes.io/service-account-token`-typed Secrets in. When set, the file is re-read on each signing request and attached as `Authorization: Bearer <token>` on the gRPC metadata. |

Plaintext remote signer transport is rejected by default. Test environments that intentionally run without TLS must opt in explicitly:

```toml
[signer]
endpoint = "http://remote-signer.test:50051"
wallet_id = "ibc-attestor-test"
allow_insecure_plaintext = true
```

## Authentication

The attestor supports optional bearer-token auth designed for in-cluster Kubernetes deployments:

- If `service_account_token_path` is unset, requests go out unauthenticated. Suitable when the signer enforces network-level isolation.
- If set, the attestor reads the file **on every signing request** and inserts `Authorization: Bearer <token>` into the gRPC metadata. This means an externally-rotated short-lived token (Kubernetes ServiceAccount projected token, Vault sidecar, etc.) is picked up without restarting the attestor.

The token file must contain a bare JWT — a single line, no quotes or JSON wrapper. An empty file or unreadable path causes the signing request to fail with `SignerError::ConfigError`.

## Signer service interface

The proto lives at [`proto/signer/signerservice.proto`](../proto/signer/signerservice.proto) and is compatible with the remote signer used by `cosmos/ibc-relayer`. A conforming service must implement:

```proto
service SignerService {
    rpc GetWallet (GetWalletRequest) returns (GetWalletResponse) {}
    rpc Sign      (SignRequest)      returns (SignResponse) {}
}
```

### What the attestor actually calls

The attestor uses **only `Sign`**. `GetWallet` is included for parity with other Cosmos Labs tools (e.g. `cosmos/ibc-relayer`, which uses it to discover wallet pubkeys and as a startup health check), but the attestor never invokes it. A remote signer service that ships with the attestor as its only client may stub `GetWallet`, though we recommend implementing it so the same signer can serve a relayer or other tooling.

### What the attestor sends

Every signing request has this shape (`signer/remote.rs:104-113`):

```proto
SignRequest {
    wallet_id = "<the configured wallet_id>",
    payload = RecoverableMessage {
        message = <33-byte tagged signing input — described below>
    }
}
```

The attestor populates only the `recoverable_message` variant of the `SignRequest.payload` `oneof`. The 33-byte message is `domain_tag || sha256(attested_data)` (where `domain_tag` is `0x01` for state attestations or `0x02` for packet attestations); the signer must `sha256` this input and produce an ECDSA secp256k1 signature over the resulting digest.

### What the attestor expects back

The response must populate the `recoverable_signature` variant of `SignResponse.signature`:

```proto
SignResponse {
    signature = RecoverableMessageSignature {
        r = <32 bytes>,
        s = <32 bytes>,
        v = <1 byte>
    }
}
```

The attestor validates strictly (`signer/remote.rs:141-151`):

- The signature variant must be `RecoverableMessageSignature`. Any other variant returns `SignerError::InvalidSignature("expected recoverable signature")`.
- `r` must be exactly 32 bytes; `s` must be exactly 32 bytes; `v` must be exactly 1 byte. Wrong lengths are rejected with `SignerError::InvalidSignature("expected r=32 s=32 v=1 bytes, got …")`.

The attestor concatenates `r || s || v` into a 65-byte recoverable signature, then parses it via `alloy_primitives::Signature::try_from`. The resulting signature is what's returned in the `Attestation.signature` field on the gRPC response.

The `v` byte must encode the recovery id in a form `alloy_primitives::Signature` accepts (either `0`/`1` or `27`/`28`). Returning a non-recoverable ECDSA signature, a different signature scheme, or any malformed value causes the signing request to fail and the attestation to be rejected.

### `GetWallet` semantics

`GetWallet` is the conformance probe of the interface — even though the attestor itself never calls it, an interface-compliant remote signer should implement it so operators can verify that a `wallet_id` resolves to the intended key before pointing a production attestor at it.

For `GetWallet(GetWalletRequest { id, pubkey_type })` the response `Wallet` must satisfy:

| Field               | Expected value |
|---------------------|----------------|
| `id`                | The `wallet_id` from the request. |
| `pubkey`            | The secp256k1 public key, encoded per `pubkey_type` (compressed 33 bytes for `Raw`/`Cosmos`/`Solana`, uncompressed 65 bytes for `Ethereum`). |
| `address_bytes`     | The chain-native address derived from `pubkey`. For `pubkey_type = Ethereum`: the 20-byte address `keccak256(uncompressed_pubkey[1:])[12:]`. |
| `formatted_address` | The canonical string form — EIP-55 mixed-case `0x…` for Ethereum, bech32 for Cosmos. |

**The address returned by `GetWallet` MUST equal the address derivable from signatures produced by `Sign` for the same `wallet_id`.** If the two diverge — e.g. the service has separate metadata-lookup and signing wallets internally — any attestation the attestor emits will fail on-chain verification.

For the attestor specifically, query with `pubkey_type = Ethereum`: the light client implementations for all chain types do Ethereum-style `ecrecover`, so that's the address you need to register on chain.

## Verifying your remote signer before deployment

Before pointing a production attestor at a remote signer, confirm end-to-end that `GetWallet` and `Sign` agree, and that the result matches what you intend to register on the light client.

1. **Look up the expected address** via `GetWallet`. Record `formatted_address`; call this `EXPECTED_ADDR`.

   ```bash
   grpcurl \
     -d '{"id": "<wallet_id>", "pubkey_type": "Ethereum"}' \
     remote-signer:50051 \
     signerservice.SignerService/GetWallet
   ```

2. **Sign a probe message** via `Sign`. The probe doesn't need to be a real attestation — any 33-byte input works. Below uses `0x01` (the state-attestation domain tag) followed by `sha256("ibc-attestor probe")`:

   ```bash
   # Build the 33-byte probe input
   INNER_HASH=$(printf '%s' "ibc-attestor probe" | shasum -a 256 | awk '{print $1}')
   PROBE_HEX="01${INNER_HASH}"
   PROBE_B64=$(printf '%s' "$PROBE_HEX" | xxd -r -p | base64)

   grpcurl \
     -d "{\"wallet_id\": \"<wallet_id>\", \"recoverable_message\": {\"message\": \"$PROBE_B64\"}}" \
     remote-signer:50051 \
     signerservice.SignerService/Sign
   ```

   The response carries `r`, `s`, `v` as separate base64-encoded byte fields under `recoverable_signature`.

3. **Recover the signer address** from the response. The signer is expected to have computed `sha256(probe)` and signed that digest, so reconstruct the same digest and `ecrecover` over `(r, s, v)`. The cleanest path is Python's [`eth_keys`](https://pypi.org/project/eth-keys/) (`pip install eth-keys`) — note that `eth_keys` only accepts `v` as `0` or `1`; values of `27`/`28` must be normalised first:

   ```bash
   PROBE_HASH=$(printf '%s' "$PROBE_HEX" | xxd -r -p | shasum -a 256 | awk '{print $1}')

   # $RESP_JSON is the grpcurl Sign response from step 2.
   echo "$RESP_JSON" | python3 -c '
   import sys, json, base64
   from eth_keys import keys
   rs = json.load(sys.stdin)["recoverableSignature"]
   v_raw = base64.b64decode(rs["v"])[0]
   v = v_raw % 27  # normalise: 0->0, 1->1, 27->0, 28->1
   sig = keys.Signature(vrs=(
       v,
       int.from_bytes(base64.b64decode(rs["r"]), "big"),
       int.from_bytes(base64.b64decode(rs["s"]), "big"),
   ))
   print(sig.recover_public_key_from_msg_hash(bytes.fromhex(sys.argv[1])).to_checksum_address())
   ' "$PROBE_HASH"
   ```

4. **Compare** the recovered address to `EXPECTED_ADDR`. They must match exactly. If they don't, the signer is non-conforming — see the pitfall table below.

5. **Register `EXPECTED_ADDR` on the on-chain light client.** Only after step 4 passes.

## Trust assumptions

- The remote signer is part of the attestor's trusted computing base. A compromised signer can produce arbitrary attestations under the registered key — equivalent to compromising the attestor host itself.
- The `wallet_id` is opaque to the attestor; the operator is responsible for ensuring it maps to the secp256k1 key whose address is registered on the on-chain light client. A misconfigured `wallet_id` produces signatures the on-chain verifier won't accept.
