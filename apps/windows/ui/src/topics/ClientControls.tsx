import { useEffect, useState, useSyncExternalStore, type ReactNode } from "react";
import type { ClientInventory, ClientItem, ClientLeftForYou, ClientReviewFlag } from "../clientControls";
import { useL10n } from "../l10n";
import {
  ClientRetentionReviewModel,
  ClientReviewModel,
  ClientSettingsModel,
  clientMessage,
  countLines,
  derivedLine,
  expiryLeftForYouLine,
  expiryStoppedLine,
  failedLine,
  gmailThreadsLine,
  groupProgress,
  groupTitle,
  heldDevicesLine,
  isRetryable,
  keepLabel,
  kindName,
  leftForYouLine,
  matchingLimitLine,
  noIdentityLine,
  pendingDevicesLine,
  reappearsLine,
  receiptHeadline,
  receiptRetryHelps,
  reviewRows,
  rowDetail,
  rollbackCopyRemovedLine,
  rowTitle,
  sharedWith,
  summaryComplete,
  type CaptureGroup,
  type ClientControlsBridge,
  type ExpirySummary,
  type ExportOutcome,
  type LeftLine,
  type Observable,
  type ReviewMode,
} from "./clientControlsModel";

/** Re-renders when `model` changes. */
function useModel<M extends Observable>(model: M): M {
  useSyncExternalStore(model.subscribe, model.getVersion);
  return model;
}

/**
 * The "Client" section on a seed topic's page (the Mac's ClientControlsView): Delete this client…, Export…, Keep for
 * N months, Exclude from hosted models. Hidden when core says the topic is not a live client.
 */
export function ClientSection({
  topicKey,
  bridge,
  openReview,
}: {
  topicKey: string;
  bridge: ClientControlsBridge;
  openReview: (mode: ReviewMode) => void;
}) {
  const { t } = useL10n();
  const [model] = useState(() => new ClientSettingsModel(topicKey, bridge));
  useModel(model);
  useEffect(() => {
    void model.load();
  }, [model]);
  if (!model.isClient || model.saved == null) return null;
  const notice = model.exclusionNotice;

  return (
    <section className="field card client-controls">
      <h3 className="section-label">{t("Client")}</h3>
      {model.errorText && <p role="alert">{model.errorText}</p>}
      <div className="toolbar">
        <button
          className="mode"
          title={t("Review what Whimbrel holds about this client, then delete it.")}
          onClick={() => openReview("delete")}
        >
          {t("Delete this client…")}
        </button>
        <button
          className="mode"
          title={t("Copy what Whimbrel holds about this client into a folder you choose.")}
          onClick={() => openReview("export")}
        >
          {t("Export…")}
        </button>
      </div>

      <label className="field">
        <span>{t("Keep for")}</span>
        <select
          className="input"
          value={model.keepMonths ?? ""}
          disabled={model.isSaving}
          onChange={(e) => void model.setKeepMonths(e.target.value === "" ? null : Number(e.target.value))}
        >
          {model.keepOptions.map((m) => (
            <option key={m ?? "off"} value={m ?? ""}>
              {keepLabel(m)}
            </option>
          ))}
        </select>
      </label>
      <p className="muted small">
        {t(
          "The period syncs to your other devices, and those that have updated apply it. Each device asks before it deletes its own copies; deleting here does not delete them elsewhere. Each item's age counts from its own date (meeting end, email date, event date, capture time). An item shared with another client that has no keep period is kept.",
        )}
      </p>
      <p className="muted small">{gmailThreadsLine()}</p>

      <label className="toggle">
        <input
          type="checkbox"
          checked={model.excludeHosted}
          disabled={model.isSaving}
          onChange={(e) => void model.setExcludeHosted(e.target.checked)}
        />
        {t("Exclude from hosted models")}
      </label>
      <p className="muted small">
        {t(
          "Material that names or involves this client is left out of what Whimbrel sends to hosted AI models. A request that names the client is answered by a local model on this PC, or refused when there is none.",
        )}
      </p>
      <p className="muted small">
        {t(
          "Whimbrel treats a recording as this client's when it comes from or overlaps one of the client's calendar events, names the client, includes one of the client's contacts, or you file it under the client. A call with none of these looks like any other recording until you file it.",
        )}
      </p>
      <p className="muted small">
        {t(
          "An item shared with another client is left out if either client is excluded. Colleagues who mainly email this client may be left out too. A calendar series once filed under this client stays left out after you unfile it.",
        )}
      </p>
      <p className="muted small">{matchingLimitLine()}</p>
      {heldDevicesLine(model.heldDevices) && <p role="alert">{heldDevicesLine(model.heldDevices)}</p>}
      {notice && (
        <div className="card notice" role="status">
          <p>{notice.alreadyLeft}</p>
          {notice.hostedSummaries.length > 0 && (
            <>
              <p className="muted small">
                {t(
                  "A hosted model summarised {n} of this client's meetings before. Those summaries stay as they are; from now on they are left out of what Whimbrel sends.",
                  { n: notice.hostedSummaries.length },
                )}
              </p>
              {notice.resummaryStarted && notice.resummaryNotMade == null ? (
                <p className="muted small">{t("Summarising them again on this PC. Without a local model they are not made again.")}</p>
              ) : notice.resummaryNotMade === 0 ? (
                <p className="muted small">{t("Summarised them again on this PC.")}</p>
              ) : notice.resummaryNotMade != null ? (
                <p role="alert">
                  {t("{n} of them were not summarised again. Open each meeting and use Regenerate summary when a local model is set up.", {
                    n: notice.resummaryNotMade,
                  })}
                </p>
              ) : (
                <button className="mode" onClick={() => void model.resummarize()}>
                  {t("Summarise them again with a local model")}
                </button>
              )}
            </>
          )}
        </div>
      )}
    </section>
  );
}

/** The weekly "items ready to review" banner on the Topics tab (keep for N months). */
export function ClientReviewBanner({ flag, review, later }: { flag: ClientReviewFlag; review: () => void; later: () => void }) {
  const { t } = useL10n();
  return (
    <div className="card notice client-banner" role="status">
      <p>
        {t("Items past their keep period are ready to review (items: {items}, clients: {clients}). Nothing is deleted until you confirm.", {
          items: flag.items,
          clients: flag.clients,
        })}
      </p>
      <div className="toolbar">
        <button className="mode mode--active" onClick={review}>
          {t("Review…")}
        </button>
        <button className="mode" onClick={later}>
          {t("Later")}
        </button>
      </div>
    </div>
  );
}

/** One "Left for you" line and the paths it names (selectable, to copy). */
function LeftForYouRow({ line }: { line: LeftLine }) {
  return (
    <div className="client-left">
      <p>{line.text}</p>
      {line.paths.map((p) => (
        <p key={p} className="mono small muted client-path">
          {p}
        </p>
      ))}
    </div>
  );
}

const leftLines = (list: ClientLeftForYou[]) => list.map(leftForYouLine).filter((l): l is LeftLine => l != null);

/**
 * The review screen for "Delete this client…" and "Export…" (spec 4.1, 6): Linked, Possible matches (unticked), Left
 * for you. Delete needs the client's name typed exactly; it then shows the receipt (counts and categories only).
 * Owned by the Topics tab, so the receipt stays when the deleted topic leaves the list.
 */
export function ClientReviewDialog({ model, onClose }: { model: ClientReviewModel; onClose: () => void }) {
  const { t } = useL10n();
  useModel(model);
  useEffect(() => {
    void model.load();
  }, [model]);
  const p = model.phase;
  const title = model.mode === "delete" ? t("Delete this client") : t("Export this client's material");

  return (
    <div className="client-review-layer">
      <div className="client-review-scrim" />
      <div className="card confirm client-review" role="dialog" aria-modal="true" aria-label={title}>
        <h2 className="display">{title}</h2>
        <div className="client-review-body">
          {p.kind === "loading" && <p className="muted">{t("Loading…")}</p>}
          {p.kind === "review" && model.inventory && <Review model={model} inv={model.inventory} />}
          {p.kind === "running" && (
            <p role="status" className="muted">
              <span className="skin-chat__spinner" aria-hidden="true" /> {model.mode === "delete" ? t("Deleting…") : t("Exporting…")}
            </p>
          )}
          {p.kind === "waiting" && (
            <p role="status" className="muted">
              <span className="skin-chat__spinner" aria-hidden="true" /> {t("Waiting for background work to finish…")}
            </p>
          )}
          {p.kind === "receipt" && <ReceiptView model={model} />}
          {p.kind === "exported" && <Exported outcome={p.outcome} />}
          {p.kind === "gone" && <p role="alert">{clientMessage("not_found")}</p>}
          {p.kind === "noIdentity" && <p role="alert">{noIdentityLine()}</p>}
          {p.kind === "failed" && <p role="alert">{clientMessage(p.error)}</p>}
        </div>
        {/* PR 875: the name field (Delete) and the export options stay in view however long the lists are. */}
        {p.kind === "review" && model.inventory && (
          <div className="client-review-pinned">
            <PinnedControls model={model} />
          </div>
        )}
        <Footer model={model} onClose={onClose} />
      </div>
    </div>
  );
}

function Review({ model, inv }: { model: ClientReviewModel; inv: ClientInventory }) {
  const { t, locale } = useL10n();
  const del = model.mode === "delete";
  const counts = countLines(inv.counts);
  const hasExports = inv.left_for_you.some((l) => typeof l === "object" && "export_folders" in l && l.export_folders.length > 0);
  return (
    <div className="stack">
      {model.notice && <p role="alert">{clientMessage(model.notice)}</p>}
      <section>
        <h3 className="section-label">{del ? t("Linked: deleted") : t("Linked: exported")}</h3>
        {counts.length === 0 ? (
          <p className="muted small">{t("Nothing is filed under this client yet.")}</p>
        ) : (
          counts.map((c) => <p key={c}>{c}</p>)
        )}
        {/* Captures are counted only: a linked episode can hold thousands. */}
        {inv.linked.some((i) => i.kind !== "capture") && (
          <details className="client-linked">
            <summary>{t("Show linked items")}</summary>
            {inv.linked
              .filter((i) => i.kind !== "capture")
              .map((i) => (
                <div key={`${i.kind}:${i.id}`} className="client-row">
                  <ItemText item={i} />
                </div>
              ))}
          </details>
        )}
        {sharedWith(inv.linked).map((other) => (
          <p key={other} className="small">
            {del
              ? t("Some of these also belong to {client}. They are deleted for both.", { client: other })
              : t("Some of these also belong to {client}. They are exported whole.", { client: other })}
          </p>
        ))}
        {del ? (
          <>
            <p className="muted small">{derivedLine()}</p>
            <p className="muted small">{gmailThreadsLine()}</p>
          </>
        ) : (
          <>
            <p className="muted small">
              {t("Never another client's own material. A meeting shared with another client is exported whole.")}
            </p>
            <p className="muted small">
              {t("Gmail thread summaries are not included. Possible matches are included only if you tick them.")}
            </p>
          </>
        )}
      </section>

      <section>
        <h3 className="section-label">{t("Possible matches")}</h3>
        {inv.possible.length === 0 ? (
          <p className="muted small">{t("No other items name this client.")}</p>
        ) : (
          <>
            <p className="muted small">
              {del
                ? t("Items that name this client but are not filed under it. Tick the ones to delete too.")
                : t("Items that name this client but are not filed under it. Tick the ones to export too.")}
            </p>
            {reviewRows(inv.possible).map((row) =>
              row.kind === "item" ? (
                <TickRow key={`${row.item.kind}:${row.item.id}`} model={model} item={row.item} />
              ) : (
                <CaptureGroupRow key={row.group.id} model={model} group={row.group} locale={locale} />
              ),
            )}
            {inv.possible_truncated && (
              <p className="muted small">
                {del
                  ? t(
                      "More items name this client than are listed here. The ones not listed are not deleted and stay in Whimbrel; delete them where they are if you need to.",
                    )
                  : t("More items name this client than are listed here. The ones not listed are not exported.")}
              </p>
            )}
          </>
        )}
      </section>

      {del ? (
        <>
          <section>
            <h3 className="section-label">{t("Left for you")}</h3>
            <p className="muted small">{t("Places Whimbrel cannot clear for you.")}</p>
            {leftLines(inv.left_for_you).map((l, i) => (
              <LeftForYouRow key={i} line={l} />
            ))}
            {hasExports && (
              <label className="toggle">
                <input type="checkbox" checked={model.deleteExports} onChange={(e) => model.setDeleteExports(e.target.checked)} />
                {t("Also delete the export folders listed above")}
              </label>
            )}
          </section>
          <p className="muted small">{reappearsLine()}</p>
        </>
      ) : null}
    </div>
  );
}

/** Below the scrolling lists: the name to type (Delete) or the export options. */
function PinnedControls({ model }: { model: ClientReviewModel }) {
  const { t } = useL10n();
  if (model.mode === "delete")
    return (
      <label className="field">
        <span>
          {t("Type the client's name to confirm: {name}", {
            name: model.name,
          })}
        </span>
        <input
          className="input"
          value={model.confirmText}
          placeholder={t("Client name")}
          aria-label={t("Type the client's name to confirm")}
          autoComplete="off"
          spellCheck={false}
          // Back from a cancelled wait or a changed list, the field is ready to confirm again.
          autoFocus={model.confirmText.length > 0}
          onChange={(e) => model.setConfirmText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void model.delete();
          }}
        />
      </label>
    );
  return (
    <>
      <label className="toggle">
        <input type="checkbox" checked={model.includeMedia} onChange={(e) => model.setIncludeMedia(e.target.checked)} />
        {t("Include audio and slide images (can be large)")}
      </label>
      <p className="muted small">
        {t("Whimbrel makes a new folder inside the folder you choose. Export to a folder on this PC, not a network share.")}
      </p>
    </>
  );
}

/** A row's text: its name, then what it is and when, then the line that names the client (PR 875). */
function ItemText({ item }: { item: ClientItem }) {
  const { t, locale } = useL10n();
  const detail = rowDetail(item, locale);
  const excerpt = item.excerpt?.trim();
  return (
    <span className="client-row-text">
      <span>{rowTitle(item, locale)}</span>
      {detail && <span className="small muted">{detail}</span>}
      {excerpt && <span className="small muted client-excerpt">{excerpt}</span>}
      {item.also_client && <span className="small">{t("Also belongs to {client}", { client: item.also_client })}</span>}
    </span>
  );
}

function TickRow({ model, item }: { model: ClientReviewModel; item: ClientItem }) {
  return (
    <label className="toggle client-row">
      <input type="checkbox" checked={model.isTicked(item)} onChange={(e) => model.setTicked(item, e.target.checked)} />
      <ItemText item={item} />
    </label>
  );
}

/** Screen captures from one app on one day: one box ticks them all; opening the row shows each with its own box. */
function CaptureGroupRow({ model, group, locale }: { model: ClientReviewModel; group: CaptureGroup; locale: string }) {
  const n = model.tickedCount(group.items);
  const all = model.isGroupTicked(group.items);
  return (
    <details className="client-group">
      <summary>
        <label className="toggle client-row" onClick={(e) => e.stopPropagation()}>
          <input
            type="checkbox"
            checked={all}
            ref={(el) => {
              if (el) el.indeterminate = n > 0 && !all;
            }}
            onChange={(e) => model.setGroupTicked(group.items, e.target.checked)}
          />
          <span className="client-row-text">
            <span>{groupTitle(group, locale)}</span>
            {n > 0 && !all && <span className="small muted">{groupProgress(n, group.items.length)}</span>}
          </span>
        </label>
      </summary>
      <div className="client-group-items">
        {group.items.map((i) => (
          <TickRow key={`${i.kind}:${i.id}`} model={model} item={i} />
        ))}
      </div>
    </details>
  );
}

function Exported({ outcome }: { outcome: ExportOutcome }) {
  const { t } = useL10n();
  return (
    <div className="stack">
      <p>
        <strong>{t("Exported.")}</strong>
      </p>
      <LeftForYouRow line={{ text: t("The export folder:"), paths: [outcome.folder] }} />
      {outcome.partial && <p role="alert">{t("The text is complete, but some audio or slide images were not copied.")}</p>}
      {outcome.media && (
        <p className="muted small">
          {t("Audio and slide images: {files} copied, {skipped} skipped, {failed} failed.", {
            files: outcome.media.files,
            skipped: outcome.media.skipped,
            failed: outcome.media.failed,
          })}
        </p>
      )}
      <p className="muted small">
        {t(
          "The folder has a README that says what is in it. It is a plain copy of private data, yours to keep or delete. File History or cloud sync may copy it too. If you delete this client later, Whimbrel lists this folder under Left for you and offers to delete it. If you move or rename it, Whimbrel loses track of it: delete it yourself.",
        )}
      </p>
    </div>
  );
}

/** The receipt: counts and categories only, never the client's name or text (spec 4.4). */
function ReceiptView({ model }: { model: ClientReviewModel }) {
  const { t } = useL10n();
  if (model.phase.kind !== "receipt") return null;
  const r = model.phase.receipt;
  const failed = failedLine(r);
  const section = (title: string, lines: string[]) => (
    <section>
      <h3 className="section-label">{title}</h3>
      {lines.length === 0 ? <p className="muted small">{t("Nothing.")}</p> : lines.map((l) => <p key={l}>{l}</p>)}
    </section>
  );
  return (
    <div className="stack">
      {model.cancelTooLate && <p className="muted small">{t("Too late to cancel. The delete had already started and finished.")}</p>}
      <p role={r.complete ? "status" : "alert"}>
        <strong>{receiptHeadline(r)}</strong>
      </p>
      {failed && <p role="alert">{failed}</p>}
      {model.retryGone && (
        <p className="muted small">
          {t("The client record is already gone, so there is nothing more to run again. What is left is listed below.")}
        </p>
      )}
      {section(t("Deleted"), countLines(r.counts))}
      {countLines(r.not_sent).length > 0 && section(t("Deleted here, not sent to your other devices"), countLines(r.not_sent))}
      {countLines(r.local_only).length > 0 && section(t("Never synced from this PC"), countLines(r.local_only))}
      {/* Only when that stage did not fail: the failed line above says so otherwise. */}
      {!r.failed_stages.includes("derived_text") && <p className="muted small">{derivedLine()}</p>}
      <p className="muted small">{gmailThreadsLine()}</p>
      {rollbackCopyRemovedLine(r) && <p className="muted small">{rollbackCopyRemovedLine(r)}</p>}
      {r.client !== "removed_here_only" && <p>{pendingDevicesLine(r.pending_devices)}</p>}
      <section>
        <h3 className="section-label">{t("Left for you")}</h3>
        {leftLines(r.left_for_you).map((l, i) => (
          <LeftForYouRow key={i} line={l} />
        ))}
      </section>
      <p className="muted small">{reappearsLine()}</p>
    </div>
  );
}

function Footer({ model, onClose }: { model: ClientReviewModel; onClose: () => void }) {
  const { t } = useL10n();
  const p = model.phase;
  const close = (primary = false) => (
    <button className={primary ? "mode mode--active" : "mode"} onClick={onClose}>
      {t("Close")}
    </button>
  );
  let buttons: ReactNode;
  if (p.kind === "review" && model.mode === "delete") {
    buttons = (
      <>
        <button className="mode" onClick={onClose}>
          {t("Cancel")}
        </button>
        <button className="mode mode--active" disabled={!model.canDelete} onClick={() => void model.delete()}>
          {t("Delete client and chosen items")}
        </button>
        {model.notice === "busy" && (
          <button className="mode" onClick={() => void model.retry()}>
            {t("Try again")}
          </button>
        )}
      </>
    );
  } else if (p.kind === "review") {
    buttons = (
      <>
        <button className="mode" onClick={onClose}>
          {t("Cancel")}
        </button>
        <button className="mode mode--active" disabled={!model.canExport} onClick={() => void model.export()}>
          {t("Choose folder and export…")}
        </button>
      </>
    );
  } else if (p.kind === "waiting" || p.kind === "running") {
    // The dialog can't be closed while a delete or an export runs. Until the delete starts, Cancel stops it with
    // nothing deleted (PR 875); after that core answers too late and the receipt says so.
    buttons =
      model.mode === "delete" ? (
        <>
          <button className="mode" onClick={() => void model.cancelWait()}>
            {t("Cancel")}
          </button>
          <button className="mode mode--active" disabled>
            {t("Delete client and chosen items")}
          </button>
        </>
      ) : null;
  } else if (p.kind === "receipt") {
    buttons = (
      <>
        {model.exportFolderUnreachable && !model.retryGone && (
          <button className="mode" onClick={() => void model.retryWithoutDeletingExports()}>
            {t("Run again without deleting exports")}
          </button>
        )}
        {/* An unreachable export folder: Try again would hit it again; the run above leaves it to the user. */}
        {receiptRetryHelps(p.receipt) && !model.retryGone && !model.exportFolderUnreachable && (
          <button className="mode" onClick={() => void model.retry()}>
            {t("Try again")}
          </button>
        )}
        <button className="mode mode--active" onClick={onClose}>
          {t("Done")}
        </button>
      </>
    );
  } else if (p.kind === "failed" && model.mode === "delete" && isRetryable(p.error)) {
    buttons = (
      <>
        {close()}
        <button className="mode mode--active" onClick={() => void model.retry()}>
          {t("Try again")}
        </button>
      </>
    );
  } else if (p.kind === "failed" && model.mode === "export") {
    buttons = (
      <>
        {close()}
        <button
          className="mode mode--active"
          onClick={() => {
            model.backToReview();
            void model.export();
          }}
        >
          {t("Choose another folder…")}
        </button>
      </>
    );
  } else {
    buttons = close(true);
  }
  return <div className="toolbar client-review-footer">{buttons}</div>;
}

/** The weekly keep-for-N-months review: each client's items past its period, ticked to delete, with Keep and Snooze
 *  per item. Deletes run on this PC only. */
export function ClientRetentionDialog({ model, onClose }: { model: ClientRetentionReviewModel; onClose: () => void }) {
  const { t, locale } = useL10n();
  useModel(model);
  useEffect(() => {
    void model.load();
  }, [model]);
  const p = model.phase;
  const title = t("Items ready to review");
  return (
    <div className="client-review-layer">
      <div className="client-review-scrim" />
      <div className="card confirm client-review" role="dialog" aria-modal="true" aria-label={title}>
        <h2 className="display">{title}</h2>
        <div className="client-review-body">
          {p.kind === "loading" && <p className="muted">{t("Loading…")}</p>}
          {p.kind === "review" && (
            <div className="stack">
              <p className="muted small">
                {t(
                  "These items are past their client's keep period. Ticked items are deleted from this PC when you confirm; each of your other devices asks before it deletes its own copies. Keep never asks again about an item; Snooze asks again in 30 days.",
                )}
              </p>
              {model.errorText && <p role="alert">{model.errorText}</p>}
              {model.groups.length === 0 && <p>{t("Nothing is waiting for review.")}</p>}
              {model.groups.map((g) => (
                <section key={g.key}>
                  <h3 className="section-label">{g.name}</h3>
                  {g.items.map((item) => (
                    <div key={`${item.kind}:${item.id}`} className="field-row">
                      <label className="toggle grow">
                        <input
                          type="checkbox"
                          checked={model.isTicked(g.key, item)}
                          onChange={(e) => model.setTicked(g.key, item, e.target.checked)}
                        />
                        <span>
                          {rowTitle(item, locale)}
                          {rowDetail(item, locale) && <span className="small muted"> · {rowDetail(item, locale)}</span>}
                          {item.also_client && (
                            <span className="small">
                              {" "}
                              ·{" "}
                              {t("Also belongs to {client}", {
                                client: item.also_client,
                              })}
                            </span>
                          )}
                        </span>
                      </label>
                      <button className="mode" onClick={() => void model.keep(g.key, item)}>
                        {t("Keep")}
                      </button>
                      <button className="mode" onClick={() => void model.snooze(g.key, item)}>
                        {t("Snooze 30 days")}
                      </button>
                    </div>
                  ))}
                </section>
              ))}
            </div>
          )}
          {p.kind === "running" && (
            <p role="status" className="muted">
              <span className="skin-chat__spinner" aria-hidden="true" /> {t("Deleting on this PC…")}
            </p>
          )}
          {p.kind === "done" && <RetentionDone s={p.summary} />}
          {p.kind === "failed" && <p role="alert">{clientMessage(p.error)}</p>}
        </div>
        <div className="toolbar client-review-footer">
          {p.kind === "review" ? (
            <>
              <button
                className="mode"
                onClick={() => {
                  void model.later().then(onClose);
                }}
              >
                {t("Later")}
              </button>
              <button className="mode mode--active" disabled={model.tickedCount === 0} onClick={() => void model.deleteTicked()}>
                {t("Delete ticked items ({n})", { n: model.tickedCount })}
              </button>
            </>
          ) : p.kind === "running" ? null : (
            <button className="mode mode--active" onClick={onClose}>
              {t("Done")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

function RetentionDone({ s }: { s: ExpirySummary }) {
  const { t } = useL10n();
  return (
    <div className="stack">
      <p>
        <strong>{summaryComplete(s) ? t("Deleted from this PC.") : t("Not finished.")}</strong>
      </p>
      {s.stopped && <p role="alert">{expiryStoppedLine(s.stopped)}</p>}
      {s.unfinished && <p role="alert">{t("Some items are still waiting. Review again to finish.")}</p>}
      {countLines(s.counts).map((l) => (
        <p key={l}>{l}</p>
      ))}
      {countLines(s.waitingToSync).length > 0 && (
        <>
          <p className="small">{t("Not yet sent to your other devices, so kept for now and offered again later:")}</p>
          {countLines(s.waitingToSync).map((l) => (
            <p key={l} className="small">
              {l}
            </p>
          ))}
        </>
      )}
      {countLines(s.notDue).length > 0 && (
        <>
          <p className="small">{t("No longer due, so kept:")}</p>
          {countLines(s.notDue).map((l) => (
            <p key={l} className="small">
              {l}
            </p>
          ))}
        </>
      )}
      {s.failed.length > 0 && (
        <p role="alert">
          {t("Not removed: {list}.", {
            list: s.failed.map(kindName).join(", "),
          })}
        </p>
      )}
      {s.leftForYou.map(expiryLeftForYouLine).filter((l): l is LeftLine => l != null).map((l, i) => (
        <LeftForYouRow key={i} line={l} />
      ))}
      <p className="muted small">{gmailThreadsLine()}</p>
    </div>
  );
}
