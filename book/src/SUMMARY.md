# Summary

[Layover](./index.md)

# Using Layover

- [Install](./install.md)
- [Your first factory](./first-factory.md)
- [The reference factory](./reference-factory.md)
- [Updating Layover](./updating.md)
- [Troubleshooting](./troubleshooting.md)

# Reference

- [The `layover` command](./cli.md)
- [Configuration](./configuration.md)
  - [Runners](./runners.md)
  - [Routes and joins](./routes.md)
- [Pipelines and triggers](./pipelines.md)
- [Prompts](./prompts.md)
- [The tools an agent has](./tools.md)
- [Cost](./cost.md)
- [The dashboard](./dashboard.md)
  - [Chains and the route map](./dashboard-chains.md)
  - [Runs, cost and retention](./dashboard-runs.md)
- [Help and learnings](./learning.md)
- [Recovery and steering](./recovery.md)
- [HTTP API](./http-api.md)
- [Glossary](./glossary.md)

# Design

- [How it works](./design.md)
  - [Architecture](./design/architecture.md)
  - [Routing](./design/routing.md)
  - [Decisions](./design/decisions.md)
    - [The shape of the system](./design/decisions/foundations.md)
    - [Routes, joins and reach](./design/decisions/routes.md)
    - [Workflows, triggers and spawning](./design/decisions/workflows.md)
    - [Prompts, flags and what a run is told](./design/decisions/prompts.md)
    - [Runners, agents and configuration](./design/decisions/runners.md)
    - [Cost, Fuel and the Reserve](./design/decisions/cost.md)
    - [History and the dashboard](./design/decisions/history-and-dashboard.md)
    - [Recovery, stopping and starting](./design/decisions/recovery.md)
    - [Running agents in parallel](./design/decisions/concurrency.md)
    - [Learnings and help requests](./design/decisions/learning.md)
    - [Updating Layover](./design/decisions/updates.md)
  - [Risks](./design/risks.md)
    - [Releases and updating](./design/risks-releases.md)
  - [Before 1.0](./design/first-release.md)