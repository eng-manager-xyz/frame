# EngManager zone ownership contract

Frame deliberately contains no Terraform resource for the shared
`engmanager.xyz` zone. One state in the portfolio infrastructure repository
must import and own each zone phase in full before adding Frame rules. Creating
a second `cloudflare_ruleset` for the same phase can replace unrelated apex,
shop, Stripe, or portfolio behavior.

`frame-contract.json` is the machine-readable handoff consumed by both
repositories' contract CI. It is not Terraform state and cannot apply a zone
mutation. `scripts/ci/check-cloudflare-zone-contract.py` fails if the handoff
widens the hostname, route, cache eligibility, purge scope, or initial security
enforcement. The portfolio state must translate it into the already imported
whole-phase rulesets and prove semantic equivalence there.

That authoritative state owns:

- exact `CNAME frame -> <frame-web>.onrender.com`, initially DNS-only and later
  proxied; no wildcard and no unrelated record edits;
- any conflicting `frame` AAAA removal after an inventory assertion;
- Full (strict), certificate/CAA verification, and HTTP-to-HTTPS behavior;
- one broad `frame.engmanager.xyz/api*` Worker Route and one query-safe
  `frame.engmanager.xyz/media-server*` compatibility fence, with Wrangler remaining
  the Worker-script deploy authority only if the route ownership is explicitly
  delegated there; the Worker owns only exact `/media-server` under the second
  prefix and returns a non-cacheable 404 for suffix lookalikes;
- bypass-first cache rules for API/auth/account/upload/finalize/health,
  cookies, authorization, mutations, private shares, WebSocket, and SSE;
- immutable caching only for fingerprinted assets and narrowly reviewed public
  share metadata, plus exact-tag/URL purge paths;
- Frame-host-scoped WAF and rate rules introduced in observe mode before
  enforcement.

Before the first mutation, generate/import every existing ruleset entrypoint
and prove a semantic no-op plan. Retire any whole-phase bootstrap script that
can overwrite the imported state. The protected apply uses least-privilege
zone credentials and production concurrency one; untrusted Frame or portfolio
pull requests receive only a credential-free plan/contract check.

Rollback is layered and exact: disable a Frame rule, remove the Worker Route,
switch the Frame CNAME to DNS-only, or restore the prior Frame record. It never
purges the entire zone, changes apex/shop records, or deletes D1/R2 data.
