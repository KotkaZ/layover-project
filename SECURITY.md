# Security policy

## Reporting a vulnerability

**Report privately through [GitHub Security Advisories][advisory]** — "Report a vulnerability" on
the Security tab. Please do not open a public issue for anything exploitable.

[advisory]: https://github.com/KotkaZ/layover-project/security/advisories/new

Expect an acknowledgement within a week. This is a single-maintainer project, so please allow
90 days before disclosing publicly, and say so if you intend to disclose sooner.

## What is supported

Only the latest release. The project is pre-1.0 and there are no backports; a fix ships in the
next tag.

## What Layover is, and why that shapes the scope

Layover is a supervisor. Its purpose is to start agent CLIs that **write to your filesystem, use
your credentials and spend your money, without a human watching**. A great deal of what it does
would be a vulnerability in another program and is the entire point of this one.

So the useful question is not "can it do something dangerous" — it is meant to — but "can it do
something dangerous that the operator did not authorise, or that the rails were supposed to
prevent".

**Not yet built:** nothing spawns a process today. There is no Tower and no MCP server. Reports
about the *design* of those are welcome and valuable, but they are design review rather than
vulnerability reports, and `docs/decisions.md` is the better place for them.

## In scope

- **Safety-rail bypass.** Hops, Fuel, the Reserve, the per-itinerary run cap, Slots or Ground
  Stop failing open, being circumvented, or reporting a figure that is not true. A rail that
  displays a number nobody can rely on is a vulnerability here, not a cosmetic bug.
- **Identity forgery.** Anything that lets a child agent successfully claim to be a different
  agent, or claim a budget it was not given. Identity comes from the Tower's per-run token and
  never from the agent — see `docs/architecture.md` §4.2.
- **Secret leakage.** Credentials reaching disk, a transcript, a help request, a report, the
  dashboard, or a log. Credentials are named in `env_from` and read from the environment; a
  literal in config should be refused at load.
- **Sandbox escape in prompt composition.** `@include` resolves within the prompt directory.
  A prompt that reads outside it — by symlink, absolute path, UNC path or otherwise — is in scope.
- **Anything reachable over the HTTP API** beyond what the operator configured, including path
  traversal onto the filesystem and stored script injection into the dashboard.
- **The release and install path.** Anything that lets a third party alter what an installer
  fetches.

## Out of scope

- **The dashboard when it is run with `--no-auth`.** That flag means what it says, and choosing it
  is choosing this. By default a token is required.
- **An agent doing something unwise within its authority.** Agents are LLMs; a factory pointed at
  a repository can change that repository. Bound what they may do with `access`, the route map and
  the rails.
- **Cost incurred inside configured budgets.** Spending money is the intended behaviour. Spending
  past a rail is not — report that.
- **Findings against a factory definition you wrote that grants an agent broad powers.** The
  configuration is the authorisation.

## Known and accepted

`docs/risks.md` records hazards that are understood and not yet mitigated — budgets metered after
the fact rather than reserved before it, external effects having no idempotency key, and the rails
bounding Layover's own runs rather than what a child does with a write-capable tool inside one.
Reports that restate those are still welcome, but they will be linked to the existing entry rather
than treated as new.
