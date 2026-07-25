/* RepoVow Cloud — shared UI helpers */
(function (global) {
  const RepoVowUI = {};

  RepoVowUI.escapeHtml = function (s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  };

  RepoVowUI.planBadge = function (plan) {
    const p = (plan || "free").toLowerCase();
    const cls = p === "pro" ? "badge pro" : "badge";
    return `<span class="${cls}">${RepoVowUI.escapeHtml(p)}</span>`;
  };

  RepoVowUI.policyBadge = function (policy) {
    if (!policy || !policy.mode) {
      return '<span class="badge muted">policy off</span>';
    }
    const label = policy.label || "unknown";
    const mode = policy.mode || "off";
    let cls = "badge";
    if (label === "valid") cls += " ok";
    else if (label === "off") cls += " muted";
    else if (policy.ok === false) cls += " warn";
    else cls += " muted";
    const title = RepoVowUI.escapeHtml(policy.detail || "");
    return `<span class="${cls}" title="${title}">policy: ${RepoVowUI.escapeHtml(label)} · ${RepoVowUI.escapeHtml(mode)}</span>`;
  };

  RepoVowUI.renderOnboarding = function (data) {
    const state = data.state || {};
    const goal = state.goal || {};
    const hasGoal = !!(goal.title && goal.title.trim());
    const hasSync =
      (state.compactions || 0) > 0 ||
      (data.changelog && data.changelog.length > 0) ||
      (data.snapshot && data.snapshot.trim().length > 20);
    const policyOk = data.policy && (data.policy.label === "valid" || data.policy.label === "off");
    const acceptanceOn =
      data.config &&
      data.config.acceptance_gate &&
      data.config.acceptance_gate.enabled;

    const items = [
      { done: true, label: "Cloud project created", hint: "You're on this dashboard" },
      {
        done: hasGoal,
        label: "Goal set",
        hint: hasGoal ? goal.title : 'Run repovow onboard or edit goal',
      },
      {
        done: hasSync,
        label: "CLI linked & synced",
        hint: hasSync ? "State received from repo" : "Run repovow cloud link + agent session",
      },
      {
        done: acceptanceOn,
        label: "Acceptance gate configured",
        hint: acceptanceOn ? data.config.acceptance_gate.command : "repovow config set --acceptance \"npm test\"",
        optional: true,
      },
      {
        done: policyOk && data.policy && data.policy.label === "valid",
        label: "Signed policy",
        hint: "repovow policy init && repovow policy sign",
        optional: true,
      },
    ];

    const done = items.filter((i) => i.done && !i.optional).length;
    const total = items.filter((i) => !i.optional).length;
    const pct = Math.round((done / total) * 100);

    let html = `<div class="onboard-head"><strong>Setup</strong><span class="muted">${done}/${total} required</span></div>`;
    html += `<div class="progress-bar"><div class="progress-fill" style="width:${pct}%"></div></div>`;
    html += '<ul class="checklist">';
    for (const item of items) {
      const icon = item.done ? "✓" : "○";
      const cls = item.done ? "done" : item.optional ? "optional" : "";
      html += `<li class="${cls}"><span class="check-icon">${icon}</span><div><div>${RepoVowUI.escapeHtml(item.label)}</div><div class="muted small">${RepoVowUI.escapeHtml(item.hint)}</div></div></li>`;
    }
    html += "</ul>";
    return html;
  };

  RepoVowUI.renderChangelog = function (events) {
    if (!events || !events.length) {
      return '<p class="muted">No events yet. Run an agent session with repovow init in your repo.</p>';
    }
    const rows = events
      .slice()
      .reverse()
      .slice(0, 25)
      .map((ev) => {
        const at = ev.at ? RepoVowUI.escapeHtml(ev.at) : "";
        const event = RepoVowUI.escapeHtml(ev.event || "event");
        const agent = ev.agent ? ` · ${RepoVowUI.escapeHtml(ev.agent)}` : "";
        const extra = ev.trigger
          ? ` <span class="muted">(${RepoVowUI.escapeHtml(ev.trigger)})</span>`
          : ev.title
            ? ` <span class="muted">— ${RepoVowUI.escapeHtml(ev.title)}</span>`
            : "";
        return `<li><time>${at}</time><span class="event-name">${event}${agent}</span>${extra}</li>`;
      })
      .join("");
    return `<ul class="timeline">${rows}</ul>`;
  };

  RepoVowUI.renderSnapshotSections = function (snapshot) {
    if (!snapshot || !snapshot.trim()) {
      return '<p class="muted">Empty snapshot — set a goal in the editor or via CLI.</p>';
    }
    const text = RepoVowUI.escapeHtml(snapshot);
    return `<pre class="snapshot-pre">${text}</pre>`;
  };

  RepoVowUI.copyButton = function (text, label) {
    const id = "copy-" + Math.random().toString(36).slice(2, 8);
    setTimeout(() => {
      const btn = document.getElementById(id);
      if (!btn) return;
      btn.onclick = () => navigator.clipboard.writeText(text);
    }, 0);
    return `<button type="button" class="btn secondary small" id="${id}">${label || "Copy"}</button>`;
  };

  global.RepoVowUI = RepoVowUI;
})(typeof window !== "undefined" ? window : globalThis);
