# The adaptor on the LEZ public testnet

What is deployed, where, and how to check it without trusting this file.

Recorded 2026-09-11 against `https://testnet.lez.logos.co`. This is what satisfies
**F7** ("BTC/USD, ETH/USD, SOL/USD, XMR/USD and ZEC/USD registered on LEZ
devnet/testnet") and the M2 half of **S1**.

## Addresses

| | |
| --- | --- |
| sequencer | `https://testnet.lez.logos.co` |
| program (guest image id) | `9166c6f3f031934e581569c544dff7136c67fdb8370b80c166823e8ae113c1f7` |
| admin account | `5b0e4f6dddea8b9ef3118d6002a25c09ab379653ffb60586f4604f1fa6a0b392` |
| admin config account | `c3e2ad897ea90ef96e3fa70d910d92f4ff7926e46dcb55525b45808ed8872d79` |

| feed | account |
| --- | --- |
| BTC | `f220052b5da9777fb92b7ddd3245772c476471561f7798a944eaabbb081ef104` |
| ETH | `deae44be6a51031e6433fdc78c825017f990b7bc9e5b207170f6bc05cc052867` |
| SOL | `9c07972fac85745fcd897d2cd1c9151ee27ee7fbfe3b931574619e89b7fa5cd8` |
| XMR | `3f67b44f9347dc4a6833cafdc3609cb65545f3ff107f6a9f82d4f8cc7fe0a640` |
| ZEC | `ef42db169602b2938cb98eb2c3391761a2d32e891bcde4367f8088cd2d67fc61` |

Each was read back after registration and decodes completely (211 of 211 bytes) as
a `FeedAccount` holding its own feed id, eight decimals, a 300 000 ms `max_age`,
five signers at a threshold of three, unpaused. All five are distinct accounts.
The config account decodes as `AdminAccount { admin: Some(<the admin above>),
pending: None }`.

## Confirming it

The program id is the guest's image id — a hash of the compiled ELF — and the
guest builds in a pinned container, so it is reproducible from source. Every
account above is a PDA of it. From this commit:

```sh
KANON_GENESIS_ADMIN=5b0e4f6dddea8b9ef3118d6002a25c09ab379653ffb60586f4604f1fa6a0b392 \
  cargo test --manifest-path e2e/Cargo.toml --no-run
```

and compare the built `kanon_methods::AGGREGATOR_ID` with the program id above. A
match means the program answering on that chain is the one in this repository; a
mismatch means the source and the deployment disagree.

The accounts can then be read with any JSON-RPC client — account ids go in as
base58:

```sh
curl -s -X POST https://testnet.lez.logos.co/ -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getAccount","params":["<base58 account id>"]}'
```

`program_owner` on each feed account is the program id above.

## Re-running it

`the_five_feeds_register_on_the_public_testnet` in `e2e/tests/sequencer_e2e.rs`:

```sh
KANON_GENESIS_ADMIN=5b0e4f6dddea8b9ef3118d6002a25c09ab379653ffb60586f4604f1fa6a0b392 \
KANON_SEQUENCER_URL=https://testnet.lez.logos.co \
  cargo test --manifest-path e2e/Cargo.toml --test sequencer_e2e \
  the_five_feeds_register_on_the_public_testnet -- --ignored --nocapture
```

It is `#[ignore]`d and asserts `KANON_SEQUENCER_URL` rather than defaulting,
because it writes to a chain other people share and cannot be reset. **CI does not
run it.** Every step is idempotent, so a re-run continues rather than repeats.

## What this is not

**The authority is a published key.** The admin account is derived from
`TEST_ADMIN_KEY`, `[0x11; 32]`, which is in this repository. `NominateAdmin` and
`AcceptAdmin` exist, so anyone reading this can take the authority permanently,
and `PauseFeed` and `DeregisterFeed` are reachable the same way. This registration
is therefore **reproducible rather than custodial**: it demonstrates that the
adaptor deploys, proves and executes on a chain we do not control. It is not a
production registration, and the operated deployment is M5's (S1, M5-01, M5-02).

Replacing the key is a **redeploy, not a migration**: the guest compiles the admin
account id in as `KANON_GENESIS_ADMIN`, so a different key means a different image
id, a different program, and different feed addresses.

**No prices are published here.** The deliverable says *registered*. Publishing is
M2-12, closed against a standalone sequencer by
`the_push_path_verifies_and_publishes_across_a_real_sequencer`, which also covers
the update path.

**The assets are placeholders.** `base_asset` and `quote_asset` carry the test's
mnemonic fill rather than LEZ account ids for real assets. The five are the USD
pairs by name — `FEEDS.md` records M2-00 confirming all five on
`redstone-primary-prod`. What 32 bytes should identify USD on LEZ is an open
question for Logos; the `twap_oracle` deployed on this same testnet uses genuine
token-definition accounts for its own pair.
