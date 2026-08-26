# The five feeds, their signers, and how often they publish

F7 names five feeds: BTC/USD, ETH/USD, SOL/USD, XMR/USD and ZEC/USD. Three of them are
safe to assume. The two privacy assets are not, so M2-00 went and looked before M2-11
registers anything.

All five are published on `redstone-primary-prod`, the data service the conformance
vectors were captured from. Each is signed by the same five signers, and a new round
lands about every ten seconds.

The figures below come from one five-minute sample on 2026-08-26, 112 polls two seconds
apart. Regenerate with:

```sh
python3 scripts/capture-signer-sets.py --minutes 5
```

## The signer set

The same five addresses sign all five feeds:

| |
|---|
| `0x51ce04be4b3e32572c4ec9135221d0691ba7d202` |
| `0x8bb8f32df04c8b654987daaed53d6b6091e3b774` |
| `0x9c5ae89c4af6aa32ce58588dbaf90d18a855b6de` |
| `0xdd682daec5a90dd295d14da4b0bec9281017b5be` |
| `0xdeb22f54738d54976c4c0fe5ce6d408e40d88499` |

Every address here was recovered from the signature over the reconstructed package and
compared against the address the gateway served beside it. Across 2,800 packages the two
agreed every time, and every package reconstructed. The recovery is the point: the roster
is the security parameter a feed is registered with, and read off the gateway's own label
it would be a claim by whoever answered the request — a wrong roster either locks a feed
out or lets a stranger in.

That the five feeds share one roster is a measurement rather than a guarantee, so the feed
config stays per feed as `verifier-core` already has it. [ADR
30](adr/0030-the-signer-set-stays-per-feed.md) records why the identical sets are stored
five times instead of collapsed into one.

### What this does not establish

A five-minute sample shows which signers *did* sign, which is not the same as which
signers *may*. Nothing here reads a roster endpoint — the set above is the set that
actually produced packages, so a signer authorised upstream but silent through the window
would not appear in it. Registering these five registers the signers observed to be live,
and M2-08's `update_signer_set` is the path for whatever the observation missed.

The rotation rate is the other half, and it is slow. These are the same five addresses the
conformance capture recorded on 2026-08-14, twelve days earlier. Against that, the older
of the two vector sources behind the conformance suite — about two and a half years back —
was signed by five addresses with no overlap at all. So the roster turns over on a scale of
years rather than weeks, which is what makes the lag between an upstream rotation and an
admin transaction a real exposure rather than a routine event: it is rare enough that
nothing exercises the path.

## The publish interval

Read from each round's own timestamp rather than from the observer's clock:

| | |
|---|---|
| rounds observed | 27 |
| intervals between them | 26 |
| nominal interval | 10,000 ms — 23 of 26 |
| intervals of 20,000 ms | 3 |
| worst observed | 20,000 ms |
| mean | 11,154 ms |

The nominal cadence is ten seconds, and it is not the number a staleness window has to
survive: three of the twenty-six intervals were double it, a round skipped each time. A
`maxAge` floor derived from the cadence alone would reject a feed that is behaving
normally.

What the sample bounds is weaker than it looks. An earlier sample the same morning, 280
seconds long, contained a 30,000 ms interval — two consecutive rounds skipped — which this
one did not. Five minutes establishes that the worst gap is at least twenty seconds, and a
longer sample finds a longer one. Any floor built on this number needs the margin to say
so.

All five feeds advance together. Every round carried the identical timestamp on all five,
and the interval sequence was the same for each, so there is one upstream publishing cycle
here and not five.

Within a round the five packages also carried one identical timestamp, in every round of
every sample — the round's, not each signer's own. The vector capture guards against the
other case anyway, skipping a feed whose packages disagree about the moment, because a
guard that has never fired is cheaper to keep than to reason about losing.
