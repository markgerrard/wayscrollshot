# DigitalOcean Spaces setup for wayscrollshot

This is a handoff for the DigitalOcean task. Set up one Space for screenshot uploads and configure this machine so wayscrollshot can use it. Do not put an access key or secret in this repository, Git, task messages, logs, or screenshots.

## Desired result

- Create a **Standard Storage** Space in `lon1` unless an existing project convention requires another region.
- Prefer the name `wayscrollshot` if it is available. Otherwise choose a short unique name and record it in the local config below.
- Enable the Spaces CDN. A custom hostname is optional and can be added later.
- Create a dedicated limited access key named `wayscrollshot-desktop` with **Read/Write/Delete** access to this Space only.
- Store the key pair in this machine's Secret Service keyring.
- Write the non-secret connection details to `~/.config/wayscrollshot/cloud.toml`.
- Upload, fetch, and delete a small test object, then remove it.

Read/Write/Delete is intentional: the app will upload screenshots and will later need to remove shared captures. Do not create an account-wide full-access key. DigitalOcean documents limited, per-Space keys here: <https://docs.digitalocean.com/products/spaces/how-to/manage-access/>.

## Space access

wayscrollshot will initially create shareable objects with the S3 `public-read` ACL and return their CDN URL. Keep bucket listing private. Do not add a bucket policy: DigitalOcean limited access keys and `PutBucketPolicy` policies are incompatible.

CORS is not required because uploads come from the native desktop app rather than browser JavaScript. Leave CORS unconfigured for now.

Use these address forms, replacing the placeholders:

```text
S3 endpoint: https://<region>.digitaloceanspaces.com
Origin URL:   https://<space>.<region>.digitaloceanspaces.com
CDN URL:      https://<space>.<region>.cdn.digitaloceanspaces.com
```

## Store credentials on this machine

`secret-tool` is installed. Store both values in the login keyring without printing them. Run the following interactively; each command reads the value silently and writes it to Secret Service.

```bash
read -rsp 'Spaces access key ID: ' WAYSCROLLSHOT_ACCESS_KEY
printf '\n'
printf '%s' "$WAYSCROLLSHOT_ACCESS_KEY" | secret-tool store \
  --label='wayscrollshot DigitalOcean Spaces access key' \
  app wayscrollshot provider digitalocean-spaces profile default kind access-key-id
unset WAYSCROLLSHOT_ACCESS_KEY

read -rsp 'Spaces secret access key: ' WAYSCROLLSHOT_SECRET_KEY
printf '\n'
printf '%s' "$WAYSCROLLSHOT_SECRET_KEY" | secret-tool store \
  --label='wayscrollshot DigitalOcean Spaces secret key' \
  app wayscrollshot provider digitalocean-spaces profile default kind secret-access-key
unset WAYSCROLLSHOT_SECRET_KEY
```

Verify that both entries exist without revealing their values:

```bash
test -n "$(secret-tool lookup app wayscrollshot provider digitalocean-spaces profile default kind access-key-id)"
test -n "$(secret-tool lookup app wayscrollshot provider digitalocean-spaces profile default kind secret-access-key)"
```

The application will read the same entries with:

```bash
secret-tool lookup app wayscrollshot provider digitalocean-spaces profile default kind access-key-id
secret-tool lookup app wayscrollshot provider digitalocean-spaces profile default kind secret-access-key
```

## Write the non-secret configuration

Create `~/.config/wayscrollshot/cloud.toml` with the real Space values:

```toml
enabled = true
provider = "digitalocean-spaces"
profile = "default"

endpoint = "https://lon1.digitaloceanspaces.com"
region = "lon1"
bucket = "wayscrollshot"
key_prefix = "screenshots"

# Use the CDN hostname, or a custom CDN hostname if one was configured.
public_base_url = "https://wayscrollshot.lon1.cdn.digitaloceanspaces.com"

# Uploaded screenshots should open directly from the copied share URL.
object_acl = "public-read"
```

Then restrict the file permissions:

```bash
chmod 700 ~/.config/wayscrollshot
chmod 600 ~/.config/wayscrollshot/cloud.toml
```

This file contains no credential values, but it should still remain outside the repository.

## Fallback only when Secret Service is unavailable

Do not use this fallback on the current machine. On a headless machine without `secret-tool`, create `~/.config/wayscrollshot/credentials.toml` and set mode `600`:

```toml
access_key_id = "REPLACE_WITH_ACCESS_KEY_ID"
secret_access_key = "REPLACE_WITH_SECRET_ACCESS_KEY"
```

Never create that file inside the wayscrollshot repository.

## Smoke test

Use any S3-compatible client already available to upload a small object under `screenshots/setup-test.txt` with `public-read`, request it through `public_base_url`, and then delete it. Do not install or persist another credential profile unless required. The expected public URL is:

```text
<public_base_url>/screenshots/setup-test.txt
```

Confirm all of the following in the handoff response without including either credential:

- Space name and region
- S3 endpoint
- public/CDN base URL
- dedicated limited key created and stored in the two Secret Service entries
- `cloud.toml` written at the path above
- upload, public fetch, and delete smoke test passed

