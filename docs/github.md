# Mira on GitHub

Once a repository is connected, Mira:

- **reviews every new pull request**, posting its findings as one review
  with comments on the changed lines;
- **works on requests**: write `@runmira-bot` in an issue or pull request comment
  and it does the task and opens a pull request.

It runs in the repository's own GitHub Actions, with your model key.
Nothing runs on Mira's side, and there's no file for you to write.

## Connect your repositories

In the web app, open **Settings → Integrations → GitHub** and click
**Connect GitHub**. GitHub asks which repositories to give the
**Runmira-bot** app access to; pick them and you're sent back to Mira. Then
click **Turn on** next to each repository Mira should work in.

Turning a repository on uses the provider, model and key you already
have in Mira:

1. stores the model key as the encrypted Actions secret `MIRA_API_KEY`;
2. stores the provider and model as the Actions variables `MIRA_PROVIDER`
   and `MIRA_MODEL`;
3. adds `.github/workflows/mira.yml`. If the default branch is protected,
   it opens a pull request with it instead; merge that to finish.

To change the model or key later, change it in Mira and click **Update**.

**Organizations:** on GitHub's page, pick your personal account or any
organization, and choose all or some of its repositories. **Add an
organization or change repositories** goes back there. If you aren't an
owner of the organization, GitHub sends the owners a request instead;
once they approve it, click **Connect GitHub** again. Every installation
you can see shows up in Mira, but you can only turn Mira on in
repositories where you're an admin (it stores an Actions secret there,
which GitHub reserves for admins); the others say **Needs admin**.

Reviews, comments and pull requests come from the Runmira-bot app, and
Mira's pull requests start your CI like anyone else's.

### Without the app

If you'd rather not install an app (or run Mira without its hosted
sign-in), connect a repository with a personal token instead: **Use a
personal access token instead** under the Connect button, or from inside
the repository:

```bash
mira github setup
```

It does the same three steps. `mira github status` shows whether a
repository is connected. The token needs:

- **Classic token:** the `repo` and `workflow` scopes.
  [Create one](https://github.com/settings/tokens/new?scopes=repo,workflow&description=Mira).
- **Fine-grained token:** Contents, Workflows, Secrets, Variables and
  Pull requests, read and write, on that repository.

You need admin access to the repository, because Actions secrets need
it. The CLI also picks up a signed-in GitHub CLI (`gh auth token`), or
asks for a token and saves it. In the web app it's the **GitHub** key
under **Settings → Search & keys**. With a token, comments come from
`github-actions[bot]` and Mira's pull requests don't start other
workflows.

### How the app works

The work still runs in each repository's own Actions, with the model
key stored there. The app's private key lives in one Supabase edge
function (`supabase/functions/github-app`), which only hands out
short-lived tokens:

- to a signed-in Mira user, for a repository in an installation they
  linked, to set it up;
- to a workflow run, which proves which repository it is with GitHub's
  OIDC identity (`id-token: write`), so no token is stored in the
  repository.

## Using it

| Where | Write | What happens |
| --- | --- | --- |
| New pull request | (nothing) | Mira reviews it. Drafts are skipped until marked ready. |
| Pull request comment | `@runmira-bot review` | Reviews it again. |
| Issue comment, or a new issue | `@runmira-bot fix the crash when the list is empty` | Mira works on a new branch and opens a pull request, then replies with the link. |
| Pull request comment or review comment | `@runmira-bot also handle the null case` | Mira works on a branch off the pull request's branch and opens a pull request into it. |

- Mira reacts 👀 when it picks up a request.
- The trigger is the app's name, `@runmira-bot`, so GitHub links it to the
  app. (`@mira` is someone else's GitHub account.) Repositories connected
  before this change still listen for `@mira`: click **Update** next to
  the repository in Mira to switch them.
- Only the repository's owners, members and collaborators can start
  tasks. Other people's comments, and comments from bots, are ignored.
- Tasks stop after 30 minutes. The pull request says what was done and
  how it was checked, and is left as a draft if the task isn't finished.
- Pull requests from forks aren't reviewed: GitHub doesn't give them
  access to the repository's secrets, so there's no model key. Mira also
  can't push to forks.

**CI on Mira's pull requests:** with the Runmira-bot app installed, Mira's
pull requests start your CI. Without it, pull requests opened with the
default Actions token don't start other workflows; pass a personal
access token as the action's `github-token` input if you need that.

## Using the action directly

The workflow Mira adds uses the `runmira/mira` action. You can also use
it in your own workflows:

```yaml
permissions:
  contents: write
  pull-requests: write
  issues: write
  id-token: write   # to act as the Runmira-bot app
# …
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
| `github-token` | `github.token` | Token used when the Runmira-bot app isn't installed. |
| `app-token-url` | the Mira service | Where the run swaps its OIDC identity for a Runmira-bot app token. Empty turns it off. |
| `trigger` | `@runmira-bot` | What people write to call Mira. |
| `allow` | `OWNER,MEMBER,COLLABORATOR` | Who can start tasks. |
| `max-runtime-minutes` | `30` | Time limit for a task. |
| `setup` | | Command to run before a task, e.g. `npm ci`. |
| `verify` | `true` | Double-check review findings (fewer false positives, slower). |
| `mira-version` | `latest` | Mira release to install. |

The action runs `mira github`, which reads the Actions event. You can run
it yourself with `--event-name` and `--event-path` to test a payload.

## Running the Mira service yourself

The app and its edge function live in this repository. To run your own:

1. Apply the migrations in `supabase/migrations/` and deploy
   `supabase/functions/github-app` (gateway JWT check off; the function
   checks callers itself) to your Supabase project.
2. Insert a one-time setup key:
   `insert into github_app_setup (key) values ('<random>');`
3. In a Mira build pointed at that project, open **Settings →
   Integrations**, enter the key, and click **Create the GitHub App**.
   GitHub creates the app from Mira's manifest and the function stores
   its credentials; nothing is copied by hand.
4. Point the action's `app-token-url` at your function.
