# Running the cloud routine runner

This is the operations runbook for the cloud routine runner. It is written for the person who
hosts it (Mike). The design is in
`docs/superpowers/specs/2026-11-15-cloud-routine-runner-design.md`.

## What the runner is

The runner is a small program, `whimbrel-cloud-runner`, packaged as a container. It acts as one
more device on a person's account. Each run it signs in, pulls the rows a routine is allowed to
read, runs the routines that person switched to "Run in the cloud", sends the results back
encrypted, and exits. It runs the daily brief, the end-of-day recap, the weekly digest and
meeting prep, so they are ready by the time the person chose even when the laptop is shut.

The promise, in the words the apps show: **Whimbrel Cloud holds your sync key. While this runs it
can read your <list>. It keeps nothing after the run.** The key sits in the host's secret store at
all times (inside the provision bundle), not only during a run. While a routine runs, the runner
reads only the rows that routine is allowed to read. The model provider sees the prompt, as with
any hosted call.

Keep these limits in plain view. Say them the same way in anything you write for users:

- The host's secret store holds the group key at all times, and the runner holds it in memory
  while it runs. "Only the allowed rows" is enforced by our code, not by cryptography. A
  compromised runner or secret store could read everything the account syncs.
- Results already made are not recalled. If a client is deleted later, a routine result the
  runner already sent stays where it is, like a result made on a laptop.
- Exclusion on the cloud matches by name, alias and contact address. Items a person filed by hand
  under a client are not known to the cloud.
- Events from your Google Calendar connection are not read by the cloud. Calendars added to your
  computer's Calendar app are. Events with a Google source, and events with no source, are
  dropped. A Google calendar added to macOS Calendar or Windows Calendar syncs as that app's
  events, so the runner reads it (it does not come through Google's API). Past routine results
  and meeting rows can still carry titles that came from a calendar. This is the accepted limit
  for version 1.

Plus and Cloud pricing is not part of this document.

## What only Mike does

Claude never runs `wrangler deploy`, `wrangler secret put`, or a live migration. These steps are
yours:

1. Apply `server/migrations/0005_runner_credential.sql` to the control-plane database.
2. Set `RUNNER_EMAILS` on the Worker: the accounts allowed to add a runner.
3. Deploy the Worker.
4. Put the runner's secrets in the host's secret store (below).

## Host requirements

- A container host that can run a job on a schedule and keep per-job secrets.
- Read-only root filesystem. The scratch store is in memory. If the host needs a writable `/tmp`,
  give it a tmpfs.
- The container runs as a non-root user (uid 10001). Do not run it as root.
- No volumes. The runner keeps nothing between runs.
- Do not set `TZ`. The runner sets it from the person's synced time zone.
- Outbound HTTPS to the sync Worker and to the account's relay database. Nothing inbound.
- Placeholder limits: 1 GB of memory and 15 minutes per run (the pull alone may take 7). These are guesses, not measured.

## Build the image

The build context is the `core/` directory:

```bash
docker build -f core/cloud-runner/Dockerfile -t whimbrel-cloud-runner core
docker run --rm --read-only whimbrel-cloud-runner --version
```

The image holds `ca-certificates` (libsql panics without native roots) and `tzdata` (for the
synced zone). CI builds the image and runs `--version` on pull requests that touch `core/`. CI
never pushes an image. Push it to your own registry yourself, if your host needs that.

## Provision one runner for one account

One container per account. Each account has its own secrets. How fast a run is depends on the
history indexing described under [When the runner is fast](#when-the-runner-is-fast).

1. **Make the runner key.** Run `init` once. It writes the secret key to a new file and prints
   only the public key:

   ```bash
   mkdir -p ./runner-out
   docker run --rm -u "$(id -u):$(id -g)" -v "$PWD/runner-out:/out" \
     whimbrel-cloud-runner init --secret-key-file /out/device-secret-key
   ```

   This one command writes a file, so it needs a writable folder on the host (the only place the
   image is run with a mount). Copy the printed public key. Move the contents of `device-secret-key` into the host
   secret store as `RUNNER_DEVICE_SECRET_KEY`, then delete the file. Never put it in a repo, a
   log, or a chat. `init` will not overwrite an existing file.

2. **Approve the runner from a real device.** On the Mac, open Settings, Devices. On Windows,
   open Settings, Sync & Devices. Choose Add Whimbrel Cloud and paste the public key. The device
   registers the runner and shows three values:
   - `RUNNER_PROVISION`
   - `RUNNER_APPROVER_PUBLIC_KEY`
   - `RUNNER_WORKER_URL`

   Copy each straight into the host secret store. Do not paste them into a log, a file in the
   repo, or a chat. Treat the provision bundle as shown once: if you lose it, see the section on a lost bundle (it needs a new runner key).

3. **Check the four values are in the secret store** and set as environment variables for the job:
   `RUNNER_DEVICE_SECRET_KEY`, `RUNNER_PROVISION`, `RUNNER_APPROVER_PUBLIC_KEY`,
   `RUNNER_WORKER_URL`. The runner's Worker credential is inside the provision bundle, so it is
   not a separate secret. `RUNNER_WORKER_URL` must be an `https://` address. The runner refuses
   plain `http://` except to `127.0.0.1`, `localhost` or `[::1]` (local tests only).

4. **Turn a routine on.** On the real device, switch "Run in the cloud" on for a routine and set
   its ready-by time. The routine must be turned on first. The toggle also needs sync on.

### The hidden switch (author-only phase)

The apps hide all of the cloud UI until a hidden switch is on. With it off and no Whimbrel Cloud
on the account, a person sees nothing new. Turn it on on the device you approve the runner from,
before step 2:

- **Mac:** `defaults write com.mpointer.kestrel com.mpointer.whimbrel.cloudRoutines.enabled -bool true`,
  then reopen Settings. (`com.mpointer.kestrel` is the app's bundle id, the defaults domain.) To
  turn it off: `defaults delete com.mpointer.kestrel com.mpointer.whimbrel.cloudRoutines.enabled`.
- **Windows:** quit the app, add `"cloudRoutines.enabled": true` to
  `%LOCALAPPDATA%\Whimbrel\data\settings.json`, and start the app again. Remove the line to turn it
  off. This key never syncs.

What the switch shows:

- Switch on: "Add Whimbrel Cloud" in Settings, and "Run in the cloud" (with its ready-by time and
  Updated or Late line) on each routine that can run in the cloud.
- Switch off, with an approved Whimbrel Cloud on the account: "Run in the cloud" shows, "Add
  Whimbrel Cloud" does not.
- Whimbrel Cloud's row in the device list always shows while it exists, and Remove always works.
- A routine already set to run in the cloud keeps its toggle, so it can be turned off.

5. **Do one run by hand** with the secrets in the environment (see the scheduling section for the
   flags). A good run prints one line of JSON and exits 0.

## Scheduling

A run with no routine set to run in the cloud is short: it reads the settings, writes the
heartbeat and exits. A run with any routine set to run in the cloud is not short, even when
nothing is due: it does the full allowlisted pull and files the pulled rows every time, and only
then finds that nothing is due. That pull reaches back up to 2 days for the daily brief and the
recap, 8 days for the weekly digest and 31 days when meeting prep is in the cloud, plus the
runner's own earlier results and every client setting.

So the schedule sets the cost. Every 5 minutes is up to 288 full pulls a day for one account,
each billed as relay rows read and compute. Proposal, until the gate measures the cost (M10 in the
gate page): run every 15 minutes (up to 96 full pulls a day). The trade-off: the runner starts a
routine only about 5 minutes before its ready-by time (p95 plus margin, placeholders now), so on a
15 minute schedule a run can start up to 15 minutes after that point and miss the ready-by time.
Set ready-by times with that slack, or keep 5 minutes and accept the cost.

```bash
docker run --rm --read-only --tmpfs /tmp --memory 1g \
  -e RUNNER_DEVICE_SECRET_KEY -e RUNNER_PROVISION \
  -e RUNNER_APPROVER_PUBLIC_KEY -e RUNNER_WORKER_URL \
  whimbrel-cloud-runner
```

(This is an example. Use your host's own scheduler and secret store, and a 15 minute kill timer.)

- Exit code 0 means the run finished. Exit code 1 means it stopped and printed a short reason on
  standard error. Exit code 2 means a bad command line.
- Retry with backoff is the host's job. The runner does not retry itself.
- Do not let two runs for the same account overlap. If your scheduler allows it, forbid
  concurrency.

### When the runner is fast

The runner pulls only the item kinds its routines read, and it can skip the rest at the relay only
for rows that carry a kind tag. Rows from before the tag existed get their tag from a one-time
history indexing that a Mac or Windows device with the updated app runs by itself while it syncs
(Settings shows "Indexing history: N%" until it is done). Until a device has finished it, the
runner behaves as before: slower, because it still reads those older rows, and still correct. Rows
pushed by devices that have not updated yet stay untagged too, so the runner is slower again (still
correct) until those devices update. The runner never does the indexing itself.

### Reasons a run stops

| Message | What it means | What to do |
|---|---|---|
| `<NAME> is not set` or `does not hold a usable value` | A secret is missing or malformed | Check the secret store |
| `RUNNER_WORKER_URL must start with https:// (plain http:// is only for this machine)` | The Worker URL is plain `http://` to another host | Use the `https://` address |
| `the provision bundle does not open with this runner's key` | Wrong key or wrong approver | Remove the runner in Settings, run `init` again for a new public key, and add it again |
| `the sync service refused this runner` | The runner was removed, the account has no approved computer, or the account email is not in `RUNNER_EMAILS` on the Worker (a 403 prints the same line) | See Revocation. Add a real device, or check `RUNNER_EMAILS` |
| `could not read the synced settings` | The settings pull failed | Retry |
| `the scratch store failed` | The in-memory store could not start | Check the memory limit, retry |
| `could not sign in to the sync service` | The Worker is down or the URL is wrong | Check `RUNNER_WORKER_URL`, retry |
| `a routine runs in the cloud but no time zone is synced` | No real device has written its zone | Open the app on a real device with sync on |
| `the synced time zone is not installed on this host` | The zone id is unknown to the image | Rebuild the image for newer tzdata |
| `the pull failed; nothing ran` | Relay read failed | Retry |
| `the pull did not read every row in time, so a client setting may be missing; nothing ran` | `ExclusionsUnread`: incomplete pull (below) | Retry; see below if it repeats |
| `a device on your account has not updated to this version, so its client settings may be held back; nothing ran` | `ExclusionsUnread`: a peer has not announced the topics capability (below) | Update or remove that device |
| `the pulled rows could not be filed under their clients; nothing ran` | `ExclusionsUnread`: filing did not finish (below) | Retry; report it if it repeats |
| `this build does not pull the client settings (a defect); nothing ran` | `ExclusionsUnread`: the allowlist lacks the client settings rows. A code defect, not an account problem | Report it; roll back to the last good image |
| `could not send the results` | The push failed | Retry. A result made but not sent is made again next run |
| `could not write the heartbeat` | The last settings write failed | Retry |

Once a routine has run, the run always sends what it made, even when a later step fails. It then
stops with that later step's reason, so a made brief reaches the devices and is not paid for again
on the next run.

### The `ExclusionsUnread` stop

Before any hosted model call, the runner must read the account's client settings, because they
say which clients are kept off hosted models. If it cannot confirm them, it stops and runs
nothing. This is on purpose. The stderr line names the cause (exit code 1 for all four):

- **Incomplete pull.** The pull did not finish its 420 second window (a large account, or a slow
  relay).
- **A device has not updated** (`PeerNotUpdated`). A device on the account has not announced the
  topics capability: an old app build, or a retired old device that never updated. It blocks runs
  until it is updated or removed in Settings, Devices. This cause is named first when it holds.
- **Filing did not finish.** The pulled rows could not all be filed under their clients, so a
  client setting may not apply.
- **The allowlist lacks the client settings rows.** A code defect in this build, never an account
  problem.

A routine that names an excluded client is skipped on the runner. The app shows it as "left for
your computer", and a computer runs it.

## Logs

The runner writes one JSON line to standard output on success (what ran, what was left for the
computer, how many rows were pulled, how long it took). It writes one short sentence to standard
error on failure. No line holds a key, a token or a setting value. The sync library can also write
relay error text to standard error, and that text may include the relay's address. Treat the logs
as sensitive operational data: keep them in your host's normal log capture with access limited to
you, and do not paste them into tickets or chats. They never hold secrets. Do not turn on debug
tracing in the host and send it anywhere that keeps request bodies.

## Revocation

- On a real device, open Settings, Devices and choose Remove next to Whimbrel Cloud.
- The account's relay tokens rotate, the same as for a lost laptop. The runner can no longer read
  or write.
- The group key the runner held is not recalled. The exposure is the same as a lost laptop. To
  fully rotate the key, the account must be reset.
- Then delete the runner's secrets from the host secret store and stop its schedule.

## Setting up again after a lost bundle

The provision bundle is shown once. If it is lost, or the register call did not finish cleanly,
registering the same runner key again returns a 409. A removed runner's key still counts as
taken, so removing it does not clear the 409. Do all three:

1. Remove the runner in Settings, Devices.
2. Run `whimbrel-cloud-runner init` again to make a new key and print a new public key.
3. Add Whimbrel Cloud again with the new public key, and replace all the runner's secrets.

If the account has no approved real device, the runner is refused. Re-register it after a real
device is approved again, for example after an account reset.

## Known limits (version 1)

- The pull window is 420 seconds (set the job's task timeout to 15 minutes so the routines still have time after it). An account too large to pull in that time never runs.
- Meeting prep lead is fixed at 30 minutes on the runner. The apps do not sync their own lead.
- The runner's profile text is not synced, so a cloud brief does not use a custom profile prompt.
- The cost per run is not measured. The memory and time limits above are placeholders.
- A run with nothing due still writes the heartbeat setting. It is the one settings write the runner makes. The runner writes it only after a successful run. A run that stops early leaves the
  last good time in place, so the apps may show "late".
- Each run starts a new sync epoch, so the runner's old relay rows may not compact. Watch relay
  size.
- Events from the Google Calendar connection are not read by the cloud now. People who use only
  that connection get cloud briefs without those events. Calendars added to the computer's
  Calendar app are read.
