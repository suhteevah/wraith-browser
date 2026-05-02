# Rotating the exposed GitHub PAT

The PAT prefix `github_pat_11AAR32JY0nItp1MVTapWL` was flagged in
[`HANDOFF.md`](../HANDOFF.md) as exposed in an earlier conversation transcript.
Audit results from 2026-05-02:

- **Repo**: clean. `git log --all -p | rg 'ghp_|github_pat_'` returns nothing.
- **Environment**: token lives in the `GH_TOKEN` environment variable on Kokonoe.
  `gh auth status` confirms it's the active token.

So the rotation is purely an environment + GitHub Settings task. No git
filter-branch or BFG run needed.

## Steps (5 minutes)

### 1. Revoke the old token

Open <https://github.com/settings/tokens> in a browser. Find the entry whose
token prefix matches `github_pat_11AAR32JY0nItp1MVTapWL`. Click **Delete**.

### 2. Generate a replacement (fine-grained, minimal scopes)

Same page, **Generate new token** → **Fine-grained**.

| Setting | Value |
|---|---|
| Token name | `kokonoe-suhteevah-2026-05-02` |
| Expiration | 90 days |
| Resource owner | suhteevah |
| Repository access | All repositories (or the specific ones you actually need from CLI) |
| Repository permissions | Contents: Read + Write; Metadata: Read; Pull requests: Read + Write; Workflows: Read |
| Account permissions | None |

Why these:

- `Contents R/W` — push commits, create releases
- `Metadata R` — required by everything else
- `Pull requests R/W` — `gh pr create`
- `Workflows R` — read-only because GH Actions is banned for this account; you never write workflow files
- everything else off — minimum-blast-radius

### 3. Install the new token

```powershell
# In an admin PowerShell on Kokonoe:
setx GH_TOKEN "github_pat_NEW_VALUE_HERE"

# Close and reopen the terminal so children see the new value, then:
gh auth status
```

Expected output: `✓ Logged in to github.com account suhteevah (GH_TOKEN)` with
the new token's prefix.

### 4. Verify nothing's reading the old token from a different file

```powershell
# Check user-level config
git config --global --get github.token
git config --get-all credential.helper

# Check that nothing else has the old token cached
gh config get -h github.com oauth_token
```

The first two should be empty (`gh` manages credentials via `GH_TOKEN`, not git
config). The third returns whatever `gh` has cached in its own config — it
should match the new PAT.

### 5. Smoke test from this repo

```powershell
cd J:\wraith-browser
gh repo view suhteevah/wraith-browser --json name,defaultBranchRef --jq .name
gh release list --limit 5    # should not 401
```

## Update HANDOFF.md after rotation

Once steps 1-5 are done, set BR-3 to ✅ in `NEXT-UP.md` and remove the TODO
line in `HANDOFF.md`.

## What I (Claude) cannot do for you

- Sign in as you on github.com to revoke the old token
- Read `setx GH_TOKEN` in your shell session — that's a secret you control
- Confirm that no copy of the old token exists outside this repo (in chat
  transcripts, in screenshots, in your password manager). Audit those yourself.
