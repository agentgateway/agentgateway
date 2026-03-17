# Contribution Guidelines

## Code Of Conduct

Read [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) before participating.

## Getting Started

1. Fork the repository on GitHub.
2. Clone your fork locally.
3. Add the upstream remote.
4. Create a topic branch for your work.

```bash
git clone https://github.com/YOUR-USERNAME/agentgateway.git
cd agentgateway
git remote add upstream https://github.com/agentgateway/agentgateway.git
git checkout -b feature/your-feature-name
```

## Development Workflow

Use the root `make` targets as the contributor interface:

```bash
make help
make bootstrap
make doctor
make test
```

[DEVELOPMENT.md](DEVELOPMENT.md) is the canonical workflow guide. It covers:

- when to run `make test`
- when to run `make env-up`
- when to run `make dev`
- when to run `make e2e`
- subsystem-native workflows

## Expectations Before Opening A PR

- Run `make lint` for check-only validation.
- Run `make test` for the default fast local verification path.
- Run `make test-proxy`, `make test-controller`, or `make test-ui` directly if your change is isolated to one subsystem.
- If your change touches the shared Kubernetes or e2e flow, validate the relevant root workflow as well: `make env-up`, `make dev`, or `make e2e`.
- Add or update tests when behavior changes.
- Update docs when the contributor workflow, API, CLI, or configuration changes.

## Commit Guidelines

We follow [Conventional Commits](https://www.conventionalcommits.org/):

- `feat`
- `fix`
- `docs`
- `style`
- `refactor`
- `perf`
- `test`
- `chore`

## Pull Request Process

1. Rebase onto the latest `upstream/main`.
2. Push your branch to your fork.
3. Open a pull request against `main`.
4. Fill out the PR template completely.
5. Address review feedback with follow-up commits.

```bash
git fetch upstream
git rebase upstream/main
git push origin feature/your-feature-name
```

## Community

- Join the [Discord server](https://discord.gg/y9efgEmppm)
- Participate in [community calls](https://calendar.google.com/calendar/u/0?cid=Y18zZTAzNGE0OTFiMGUyYzU2OWI1Y2ZlOWNmOWM4NjYyZTljNTNjYzVlOTdmMjdkY2I5ZTZmNmM5ZDZhYzRkM2ZmQGdyb3VwLmNhbGVuZGFyLmdvb2dsZS5jb20)
- Review pull requests and help answer issues

## License

By contributing to this project, you agree that your contributions are licensed under the project license.
