# Mira on GitHub

Once a repository is connected, Mira:

- **reviews every new pull request**, posting its findings as one review
  with comments on the changed lines;
- **works on requests**: write `@mira` in an issue or pull request comment
  and it does the task and opens a pull request.

It runs in the repository's own GitHub Actions, with your model key.
Nothing runs on Mira's side, and there's no file for you to write.

## Connect a repository

**In the web app:** **Settings → Integrations → GitHub**, check the
repository name, and click **Connect**.

**Or in the terminal**, from inside the repository:

```bash
mira github setup
```

Either way, Mira uses the provider, model and key you already have and:

1. stores the model key as the encrypted Actions secret `MIRA_API_KEY`;
2. stores the provider and model as the Actions variables `MIRA_PROVIDER`
   and `MIRA_MODEL`;
3. adds `.github/workflows/mira.yml`. If the default branch is protected,
   it opens a pull request with it instead; merge that to finish.

To change the model or key later, change it in Mira and connect again.
`mira github status` shows whether a repository is connected.

### The GitHub token

Connecting needs a GitHub token that can manage the repository:

- **Classic token:** the `repo` and `workflow` scopes.
  [Create one](https://github.com/settings/tokens/new?scopes=repo,workflow&description=Mira).
- **Fine-grained token:** Contents, Workflows, Secrets, Variables and
  Pull requests, read and write, on that repository.

You need admin access to the repository, because Actions secrets need
it. The CLI also picks up a signed-in GitHub CLI (`gh auth token`), or
asks for a token and saves it. In the web app it's the **GitHub** key
under **Settings → Search & keys**.

## Using it

| Where | Write | What happens |
| --- | --- | --- |
| New pull request | (nothing) | Mira reviews it. Drafts are skipped until marked ready. |
| Pull request comment | `@mira review` | Reviews it again. |
| Issue comment, or a new issue | `@mira fix the crash when the list is empty` | Mira works on a new branch and opens a pull request, then replies with the link. |
| Pull request comment or review comment | `@mira also handle the null case` | Mira works on a branch off the pull request's branch and opens a pull request into it. |

- Mira reacts 👀 when it picks up a request.
- Only the repository's owners, members and collaborators can start
  tasks. Other people's comments, and comments from bots, are ignored.
- Tasks stop after 30 minutes. The pull request says what was done and
  how it was checked, and is left as a draft if the task isn't finished.
- Pull requests from forks aren't reviewed: GitHub doesn't give them
  access to the repository's secrets, so there's no model key. Mira also
  can't push to forks.

**CI on Mira's pull requests:** pull requests opened with the default
Actions token don't start other workflows. To have your CI run on them,
use a personal access token or a GitHub App token as the action's
`github-token` input.

## Using the action directly

The workflow Mira adds uses the `runmira/mira` action. You can also use
it in your own workflows:

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0
- uses: runmira/mira@main
  with:
    api-key: ${{ secrets.MIRA_API_KEY }}
    provider: openrouter
    model: anthropic/claude-sonnet-4.5
```

| Input | Default | What it does |
| --- | --- | --- |
| `api-key` | (required) | Your model provider's API key. |
| `provider` | `openrouter` | Provider name: `openrouter`, `anthropic`, `openai`, `groq`, … |
| `model` | (required) | Model id. |
| `base-url` | | Endpoint, for providers Mira doesn't know by name. |
| `github-token` | `github.token` | Token used to comment, push and open pull requests. |
| `trigger` | `@mira` | What people write to call Mira. |
| `allow` | `OWNER,MEMBER,COLLABORATOR` | Who can start tasks. |
| `max-runtime-minutes` | `30` | Time limit for a task. |
| `setup` | | Command to run before a task, e.g. `npm ci`. |
| `verify` | `true` | Double-check review findings (fewer false positives, slower). |
| `mira-version` | `latest` | Mira release to install. |

The action runs `mira github`, which reads the Actions event. You can run
it yourself with `--event-name` and `--event-path` to test a payload.
