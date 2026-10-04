// File explorers: files INSIDE the evidence image, and files on this computer.
// Both share one component: sidebar | sortable list | preview (Info, Hex,
// Text, Strings, View). Everything inside the image is read-only.

import { toast, openModal } from "../menu.js";

const esc = (v) =>
  String(v ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const fmtSize = (b) => {
  if (b == null) return "";
  if (b < 1024) return `${b} B`;
  const u = ["KiB", "MiB", "GiB", "TiB"];
  let v = b / 1024, i = 0;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(v < 10 ? 2 : 1)} ${u[i]}`;
};
async function getJSON(url, opts) {
  const r = await fetch(url, opts);
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}
const postJSON = (url, body) =>
  getJSON(url, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
const qs = (o) => Object.entries(o).filter(([, v]) => v != null && v !== "").map(([k, v]) => `${k}=${encodeURIComponent(v)}`).join("&");

const ICONS = {
  folder: "📁", deleted: "🗑", evidence: "🧾", "evidence segment": "🧾", "disk image": "💽", video: "🎞",
  picture: "🖼", pdf: "📄", text: "📝", database: "🗄", archive: "🗜", slot: "🎥", file: "📄",
};

// ---------------------------------------------------------------------------
// Generic explorer
// ---------------------------------------------------------------------------

function createExplorer(root, src) {
  root.innerHTML = `
    <div class="xp">
      <aside class="xp-side"></aside>
      <section class="xp-main">
        <div class="xp-bar">
          <button class="xp-btn" data-act="up" title="Up (Backspace)">↑</button>
          <button class="xp-btn" data-act="refresh" title="Refresh">⟳</button>
          <div class="xp-crumbs"></div>
          <input class="xp-filter" placeholder="Filter…" spellcheck="false">
          <label class="xp-toggle"><input type="checkbox" class="xp-opt"> <span>${esc(src.optionLabel)}</span></label>
        </div>
        <div class="xp-list"><table><thead><tr>${src.columns
          .map((c) => `<th data-key="${c.key}" style="${c.width ? `width:${c.width}` : ""}">${esc(c.label)}</th>`)
          .join("")}</tr></thead><tbody></tbody></table></div>
        <div class="xp-status"></div>
      </section>
      <section class="xp-preview"><div class="xp-empty">Select a file to preview it</div></section>
    </div>`;
  const $ = (s) => root.querySelector(s);
  const ctl = {
    loc: null, entries: [], sort: { key: "name", dir: 1 }, selected: null, hexOffset: 0, src,
    async go(loc) {
      ctl.loc = loc;
      ctl.selected = null;
      $(".xp-preview").innerHTML = `<div class="xp-empty">Select a file to preview it</div>`;
      $(".xp-crumbs").innerHTML = src.crumbs(loc).map((c, i) => `<span class="xp-crumb" data-i="${i}">${esc(c.label)}</span>`).join(`<span class="xp-sep">›</span>`);
      $(".xp-crumbs").querySelectorAll(".xp-crumb").forEach((n) => (n.onclick = () => ctl.go(src.crumbs(loc)[+n.dataset.i].loc)));
      $(".xp-status").textContent = "loading…";
      try {
        ctl.entries = await src.list(loc, $(".xp-opt").checked);
        ctl.render();
      } catch (e) {
        $("tbody").innerHTML = `<tr><td colspan="9" class="empty">${esc(e.message)}</td></tr>`;
        $(".xp-status").textContent = "";
      }
    },
    render() {
      const f = $(".xp-filter").value.toLowerCase();
      const { key, dir } = ctl.sort;
      const rows = ctl.entries
        .filter((e) => !f || e.name.toLowerCase().includes(f))
        .sort((a, b) => (b.isDir - a.isDir) || (key === "size" ? (a.size - b.size) * dir : String(a[key] ?? "").localeCompare(String(b[key] ?? "")) * dir));
      $("tbody").innerHTML = rows.length
        ? rows.map((e) => `<tr class="${e.deleted ? "xp-del" : ""}" data-i="${ctl.entries.indexOf(e)}" tabindex="0">${src.columns.map((c) => `<td class="${c.cls || ""}">${c.cell(e)}</td>`).join("")}</tr>`).join("")
        : `<tr><td colspan="9" class="empty">empty folder</td></tr>`;
      const dirs = rows.filter((e) => e.isDir).length;
      const del = rows.filter((e) => e.deleted).length;
      $(".xp-status").textContent = `${rows.length} items · ${dirs} folders${del ? ` · ${del} deleted` : ""}`;
      $("tbody").querySelectorAll("tr[data-i]").forEach((tr) => {
        const e = ctl.entries[+tr.dataset.i];
        tr.onclick = () => ctl.select(e, tr);
        tr.ondblclick = () => (e.isDir ? ctl.go(src.child(ctl.loc, e)) : null);
        tr.onkeydown = (ev) => {
          if (ev.key === "Enter") e.isDir ? ctl.go(src.child(ctl.loc, e)) : ctl.select(e, tr);
          if (ev.key === "ArrowDown") tr.nextElementSibling?.focus(), tr.nextElementSibling?.click();
          if (ev.key === "ArrowUp") tr.previousElementSibling?.focus(), tr.previousElementSibling?.click();
          if (ev.key === "Backspace") ctl.up();
        };
      });
    },
    up() {
      const p = src.parent(ctl.loc);
      if (p) ctl.go(p);
    },
    async select(e, tr) {
      root.querySelectorAll("tbody tr.sel").forEach((x) => x.classList.remove("sel"));
      tr?.classList.add("sel");
      ctl.selected = e;
      ctl.hexOffset = 0;
      const pv = $(".xp-preview");
      pv.innerHTML = `
        <div class="xp-pv-head">
          <div class="xp-pv-name">${src.icon(e)} ${esc(e.name)}</div>
          <div class="xp-pv-sub">${e.isDir ? "folder" : fmtSize(e.size)}${e.deleted ? ` · <span class="xp-badge del">deleted</span>` : ""}${e.recoverable === false ? ` · <span class="xp-badge warn">content not recoverable</span>` : ""}</div>
          <div class="xp-actions"></div>
        </div>
        <div class="xp-tabs">${["Info", "Hex", "Text", "Strings", "View"].map((t, i) => `<button class="xp-tab${i ? "" : " on"}" data-t="${t}">${t}</button>`).join("")}</div>
        <div class="xp-pv-body"></div>`;
      const acts = src.actions(e, ctl.loc, ctl);
      pv.querySelector(".xp-actions").innerHTML = acts.map((a, i) => `<button class="xp-act" data-i="${i}">${esc(a.label)}</button>`).join("");
      pv.querySelectorAll(".xp-act").forEach((b) => (b.onclick = () => acts[+b.dataset.i].run()));
      pv.querySelectorAll(".xp-tab").forEach((b) => (b.onclick = () => {
        pv.querySelectorAll(".xp-tab").forEach((x) => x.classList.toggle("on", x === b));
        ctl.tab(b.dataset.t);
      }));
      if (e.isDir) pv.querySelectorAll('.xp-tab:not([data-t="Info"])').forEach((b) => (b.disabled = true));
      ctl.tab("Info");
    },
    async tab(t) {
      const e = ctl.selected;
      const body = $(".xp-pv-body");
      body.innerHTML = `<div class="xp-empty">loading…</div>`;
      try {
        if (t === "Info") {
          const rows = await src.info(e, ctl.loc);
          body.innerHTML = `<table class="xp-kv">${rows.map(([k, v]) => `<tr><th>${esc(k)}</th><td>${v}</td></tr>`).join("")}</table>`;
        } else if (t === "Hex" || t === "Text") {
          const d = await src.read(e, ctl.loc, ctl.hexOffset, 4096, "");
          const size = e.size || 0;
          body.innerHTML = `
            <div class="xp-pager">
              <button data-p="first">⏮</button><button data-p="prev">◀</button>
              <span>${ctl.hexOffset.toLocaleString()} – ${(ctl.hexOffset + d.len).toLocaleString()} of ${size.toLocaleString()}</span>
              <button data-p="next">▶</button><button data-p="last">⏭</button>
              <input class="xp-jump" placeholder="offset (dec or 0x…)">
            </div>
            <pre class="xp-pre">${esc(t === "Hex" ? d.hex : d.text) || "(empty)"}</pre>`;
          body.querySelectorAll(".xp-pager button").forEach((b) => (b.onclick = () => {
            const last = Math.max(0, Math.floor((size - 1) / 4096) * 4096);
            ctl.hexOffset = { first: 0, prev: Math.max(0, ctl.hexOffset - 4096), next: Math.min(last, ctl.hexOffset + 4096), last }[b.dataset.p];
            ctl.tab(t);
          }));
          body.querySelector(".xp-jump").onkeydown = (ev) => {
            if (ev.key !== "Enter") return;
            const v = ev.target.value.trim();
            const n = v.startsWith("0x") ? parseInt(v, 16) : parseInt(v, 10);
            if (!isNaN(n)) { ctl.hexOffset = Math.max(0, Math.floor(n / 16) * 16); ctl.tab(t); }
          };
        } else if (t === "Strings") {
          const d = await src.read(e, ctl.loc, 0, 0, "strings");
          body.innerHTML = d.strings.length
            ? `<pre class="xp-pre">${d.strings.map(([o, s]) => `<span class="xp-off">${o.toString(16).padStart(8, "0")}</span>  ${esc(s)}`).join("\n")}</pre>`
            : `<div class="xp-empty">no readable strings in the first 4 MiB</div>`;
        } else if (t === "View") {
          body.innerHTML = await src.view(e, ctl.loc, ctl);
          body.querySelector("[data-play]")?.addEventListener("click", async (ev) => {
            ev.target.disabled = true;
            ev.target.textContent = "decoding… (a few seconds)";
            try {
              const r = await src.play(e, ctl.loc);
              body.innerHTML = `<video class="xp-video" controls autoplay src="${esc(r.url)}"></video>
                <p class="muted">H.264 viewing copy (${r.fps ? r.fps.toFixed(1) + " fps est." : ""}), saved to ${esc(r.mp4)}. Not evidence.</p>`;
            } catch (err) {
              body.innerHTML = `<div class="xp-empty">${esc(err.message)}</div>`;
            }
          });
        }
      } catch (err) {
        body.innerHTML = `<div class="xp-empty">${esc(err.message)}</div>`;
      }
    },
  };
  $(".xp-filter").oninput = () => ctl.render();
  $(".xp-opt").checked = src.optionDefault;
  $(".xp-opt").onchange = () => ctl.go(ctl.loc);
  root.querySelector('[data-act="up"]').onclick = () => ctl.up();
  root.querySelector('[data-act="refresh"]').onclick = () => ctl.go(ctl.loc);
  root.querySelectorAll("thead th").forEach((th) => (th.onclick = () => {
    ctl.sort = { key: th.dataset.key, dir: ctl.sort.key === th.dataset.key ? -ctl.sort.dir : 1 };
    ctl.render();
  }));
  ctl.side = $(".xp-side");
  return ctl;
}

// ---------------------------------------------------------------------------
// Inside the evidence image
// ---------------------------------------------------------------------------

let imageCtl = null;
let imageInfo = null;

const imageSource = {
  optionLabel: "Show deleted",
  optionDefault: true,
  columns: [
    { key: "name", label: "Name", cell: (e) => `${imageSource.icon(e)} ${esc(e.name)}` },
    { key: "size", label: "Size", width: "90px", cls: "num", cell: (e) => (e.isDir ? "" : fmtSize(e.size)) },
    { key: "modified", label: "Modified", width: "150px", cell: (e) => esc(e.modified || "") },
    { key: "extra", label: "Created / changed", width: "150px", cell: (e) => esc(e.extra || "") },
    { key: "id", label: "Cluster / inode", width: "120px", cell: (e) => `<span class="muted">${esc(e.id)}</span>` },
    { key: "flags", label: "", width: "90px", cell: (e) => (e.deleted ? `<span class="xp-badge del">deleted</span>` : "") },
  ],
  icon: (e) => (e.deleted ? ICONS.deleted : e.isDir ? ICONS.folder : /^file\d{4}\.dat$/i.test(e.name) ? ICONS.slot : ICONS[guessKind(e.name)] || ICONS.file),
  crumbs(loc) {
    if (!loc) return [];
    const p = imageInfo?.partitions.find((x) => x.index === loc.part);
    const out = [{ label: `Partition ${loc.part} (${p ? p.fs : "?"})`, loc: { part: loc.part, path: [] } }];
    loc.path.forEach((seg, i) => out.push({ label: seg.name, loc: { part: loc.part, path: loc.path.slice(0, i + 1) } }));
    return out;
  },
  node: (loc) => (loc.path.length ? loc.path[loc.path.length - 1].node : "root"),
  child: (loc, e) => ({ part: loc.part, path: [...loc.path, { name: e.name, node: e.node }] }),
  parent: (loc) => (loc && loc.path.length ? { part: loc.part, path: loc.path.slice(0, -1) } : null),
  displayPath: (loc, e) => "/" + [...loc.path.map((s) => s.name), e.name].join("/"),
  async list(loc, showDeleted) {
    const d = await getJSON(`/api/x/list?${qs({ part: loc.part, node: imageSource.node(loc) })}`);
    return d.entries
      .filter((e) => showDeleted || !e.deleted)
      .map((e) => ({
        ...e, isDir: e.is_dir,
        extra: e.deleted_at ? `deleted ${e.deleted_at}` : e.created || e.changed || "",
      }));
  },
  async info(e, loc) {
    const rows = [
      ["Path", esc(imageSource.displayPath(loc, e))],
      ["Type", e.isDir ? "folder" : esc(guessKind(e.name))],
      ["Size", e.isDir ? "-" : `${fmtSize(e.size)} (${(e.size || 0).toLocaleString()} bytes)`],
      ["Modified", esc(e.modified || "-")],
      ["Created", esc(e.created || "-")],
      ["Accessed", esc(e.accessed || "-")],
      ["Changed", esc(e.changed || "-")],
      ["Deleted at", esc(e.deleted_at || "-")],
      ["Time basis", esc(e.time_basis)],
      ["Identifier", esc(e.id)],
      ["Status", e.deleted ? `<span class="xp-badge del">deleted</span> ${esc(e.note || "")}` : e.note ? esc(e.note) : "allocated"],
    ];
    if (e.isDir) return rows;
    const i = await getJSON(`/api/x/info?${qs({ part: loc.part, node: e.node, name: e.name })}`);
    e.size = i.size;
    rows[1][1] = esc(i.kind);
    rows.push(["First disk sector", i.first_sector != null ? i.first_sector.toLocaleString() : "-"]);
    rows.push(["Fragments", `${i.extent_count}${i.contiguous ? " (contiguous)" : ""}`]);
    if (i.juan_slot) {
      const s = i.juan_slot;
      rows.push(["Recording starts", `${esc(s.start_utc)} <span class="muted">(unix ${s.start_unix})</span>`]);
      rows.push(["Recording ends", `${esc(s.end_utc)} <span class="muted">(unix ${s.end_unix})</span>`]);
      rows.push(["Duration", `${s.seconds} s`]);
    }
    if (i.extents?.length) {
      rows.push(["Disk extents", `<div class="xp-ext">${i.extents.map((x) => `file +${x.file_offset.toLocaleString()} → disk ${x.disk_offset.toLocaleString()} (${fmtSize(x.len)})`).join("<br>")}${i.extent_count > i.extents.length ? "<br>…" : ""}</div>`]);
    }
    e._info = i;
    return rows;
  },
  read: (e, loc, offset, len, mode) => getJSON(`/api/x/read?${qs({ part: loc.part, node: e.node, offset, len, mode })}`),
  rawUrl: (e, loc) => `/api/x/raw?${qs({ part: loc.part, node: e.node, name: e.name })}`,
  async view(e, loc) {
    const i = e._info || (await getJSON(`/api/x/info?${qs({ part: loc.part, node: e.node, name: e.name })}`));
    if (i.playable) {
      const s = i.juan_slot;
      return `<div class="xp-viewcard">
        <div class="xp-big">${ICONS.slot}</div>
        <p>${s ? `Recording slot: ${esc(s.start_utc)} → ${esc(s.end_utc)} (${s.seconds} s)` : "Raw video stream"}</p>
        <button class="xp-act" data-play>Decode & play</button>
        <p class="muted">Makes an H.264 viewing copy of this slot in the output folder.</p></div>`;
    }
    return viewFor(i.mime || "", imageSource.rawUrl(e, loc), e.name);
  },
  play: (e, loc) => postJSON("/api/x/play", { part: loc.part, node: e.node, name: e.name }),
  actions(e, loc) {
    if (e.isDir) return [{ label: "Open", run: () => imageCtl.go(imageSource.child(loc, e)) }];
    const a = [{
      label: "Extract to output folder",
      run: async () => {
        try {
          const r = await postJSON("/api/x/extract", { part: loc.part, node: e.node, name: e.name, path: imageSource.displayPath(loc, e) });
          openModal("File extracted", `<table class="xp-kv">
            <tr><th>Saved to</th><td>${esc(r.output)}</td></tr><tr><th>Bytes</th><td>${r.bytes.toLocaleString()}</td></tr>
            <tr><th>MD5</th><td>${esc(r.md5)}</td></tr><tr><th>SHA-256</th><td>${esc(r.sha256)}</td></tr></table>
            <p class="muted">Logged in extracted/extraction_log.jsonl and in the activity log.</p>`);
        } catch (err) { toast(err.message, true); }
      },
    }];
    if (!e.recoverable) a[0].label = "Extract (content may be wrong)";
    a.push({ label: "Copy path", run: () => copy(imageSource.displayPath(loc, e)) });
    return a;
  },
};

async function renderImageSide() {
  const side = imageCtl.side;
  try {
    imageInfo = await getJSON("/api/x/image");
  } catch (e) {
    side.innerHTML = `<div class="xp-side-empty">No evidence open.<br><br>Pick an image in <b>Local Disk</b> or the Files panel, then click Analyze.</div>`;
    imageCtl.root.querySelector("tbody").innerHTML = `<tr><td colspan="9" class="empty">No evidence open</td></tr>`;
    return false;
  }
  const i = imageInfo;
  const a = i.acquisition || {};
  const name = i.image.split("/").pop();
  side.innerHTML = `
    <div class="xp-card">
      <div class="xp-card-title">🧾 ${esc(name)}</div>
      <div class="xp-card-row"><span>Format</span><b>${esc(i.kind.toUpperCase())}</b></div>
      <div class="xp-card-row"><span>Media</span><b>${fmtSize(i.media_bytes)}</b></div>
      <div class="xp-card-row"><span>Sectors</span><b>${i.sectors.toLocaleString()}</b></div>
      ${i.stored?.md5 ? `<div class="xp-card-row"><span>MD5</span><b class="mono" title="${esc(i.stored.md5)}">${esc(i.stored.md5.slice(0, 16))}…</b></div>` : ""}
      ${a.software ? `<div class="xp-card-row"><span>Imaged with</span><b>${esc(a.software)}</b></div>` : ""}
      ${a.acquired ? `<div class="xp-card-row"><span>Acquired</span><b>${esc(a.acquired)}</b></div>` : ""}
      ${a.segments?.length ? `<div class="xp-card-row"><span>Segments</span><b>${a.segments.length}</b></div>` : ""}
    </div>
    <div class="xp-side-h">Partitions</div>
    ${i.partitions.map((p) => `
      <div class="xp-part${p.browsable ? "" : " off"}" data-part="${p.index}">
        <div><b>#${p.index}</b> ${esc(p.fs)} <span class="muted">${fmtSize(p.bytes)}</span></div>
        <div class="muted">LBA ${p.start_lba.toLocaleString()}${p.volume?.label?.trim() ? ` · ${esc(p.volume.label.trim())}` : ""}</div>
        ${p.browsable ? "" : `<div class="muted">${esc(p.error || "not browsable")}</div>`}
      </div>`).join("")}`;
  side.querySelectorAll(".xp-part").forEach((n) => {
    const p = i.partitions.find((x) => x.index === +n.dataset.part);
    if (p.browsable) n.onclick = () => {
      side.querySelectorAll(".xp-part").forEach((x) => x.classList.toggle("on", x === n));
      imageCtl.go({ part: p.index, path: [] });
    };
  });
  return true;
}

export async function refreshImageExplorer() {
  if (!imageCtl) return;
  const ok = await renderImageSide();
  if (!ok) return;
  const first = imageInfo.partitions.find((p) => p.fs === "FAT32" && p.browsable) || imageInfo.partitions.find((p) => p.browsable);
  if (first && (!imageCtl.loc || imageCtl.lastImage !== imageInfo.image)) {
    imageCtl.lastImage = imageInfo.image;
    imageCtl.side.querySelector(`.xp-part[data-part="${first.index}"]`)?.classList.add("on");
    imageCtl.go({ part: first.index, path: [] });
  }
}

// ---------------------------------------------------------------------------
// Local disk
// ---------------------------------------------------------------------------

let localCtl = null;

const localSource = {
  optionLabel: "Show hidden",
  optionDefault: false,
  columns: [
    { key: "name", label: "Name", cell: (e) => `${localSource.icon(e)} ${esc(e.name)}` },
    { key: "size", label: "Size", width: "90px", cls: "num", cell: (e) => (e.isDir ? "" : fmtSize(e.size)) },
    { key: "modified", label: "Modified (UTC)", width: "140px", cell: (e) => esc(e.modified || "") },
    { key: "kind", label: "Kind", width: "120px", cell: (e) => (e.kind === "evidence" ? `<span class="xp-badge ev">evidence</span>` : `<span class="muted">${esc(e.kind)}</span>`) },
  ],
  icon: (e) => (e.isDir ? ICONS.folder : ICONS[e.kind] || ICONS.file),
  crumbs(loc) {
    const parts = loc.path.split("/").filter(Boolean);
    return [{ label: "/", loc: { path: "/" } }, ...parts.map((p, i) => ({ label: p, loc: { path: "/" + parts.slice(0, i + 1).join("/") } }))];
  },
  child: (loc, e) => ({ path: e.path }),
  parent: (loc) => (loc.path === "/" ? null : { path: loc.path.replace(/\/[^/]+\/?$/, "") || "/" }),
  async list(loc, hidden) {
    const d = await getJSON(`/api/local/list?${qs({ path: loc.path, hidden: hidden ? 1 : "" })}`);
    loc.path = d.path;
    return d.entries.map((e) => ({ ...e, isDir: e.is_dir }));
  },
  async info(e) {
    const rows = [
      ["Path", esc(e.path)],
      ["Kind", esc(e.kind)],
      ["Size", e.isDir ? "-" : `${fmtSize(e.size)} (${e.size.toLocaleString()} bytes)`],
      ["Modified (UTC)", esc(e.modified || "-")],
    ];
    if (!e.isDir && e.size > 0) {
      const d = await getJSON(`/api/local/read?${qs({ path: e.path, len: 64 })}`);
      rows.push(["Detected as", esc(d.kind)]);
    }
    return rows;
  },
  read: (e, loc, offset, len, mode) => getJSON(`/api/local/read?${qs({ path: e.path, offset, len, mode })}`),
  rawUrl: (e) => `/api/local/raw?${qs({ path: e.path })}`,
  async view(e) {
    if (e.kind === "video") return `<video class="xp-video" controls src="/api/video?${qs({ path: e.path })}"></video>`;
    if (e.kind === "evidence" || e.kind === "disk image")
      return `<div class="xp-viewcard"><div class="xp-big">${ICONS.evidence}</div><p>Evidence image</p><p class="muted">Use “Open as evidence” to analyze it.</p></div>`;
    const mime = { picture: "image/", pdf: "application/pdf", text: "text/plain" }[e.kind] || "";
    return viewFor(mime, localSource.rawUrl(e), e.name);
  },
  actions(e) {
    if (e.isDir) return [{ label: "Open", run: () => localCtl.go({ path: e.path }) }];
    const a = [];
    if (e.kind === "evidence" || e.kind === "disk image")
      a.push({ label: "Open as evidence", run: () => openEvidence(e.path) });
    if (e.kind === "video")
      a.push({ label: "Load in Viewer", run: async () => {
        try {
          await postJSON("/api/load-video", { path: e.path });
          document.querySelector('[data-view="viewer"]')?.click();
          document.dispatchEvent(new CustomEvent("video-loaded"));
        } catch (err) { toast(err.message, true); }
      } });
    a.push({ label: "Copy path", run: () => copy(e.path) });
    return a;
  },
};

async function renderLocalSide() {
  const r = await getJSON("/api/local/roots");
  const item = ([label, path], icon) => `<div class="xp-place" data-path="${esc(path)}">${icon} ${esc(label)}</div>`;
  localCtl.side.innerHTML = `
    <div class="xp-side-h">Places</div>${r.places.map((p) => item(p, p[0] === "SATYA output" ? "📦" : "📁")).join("")}
    ${r.drives.length ? `<div class="xp-side-h">Drives</div>${r.drives.map((d) => item(d, "💽")).join("")}` : ""}
    <div class="xp-side-h">Go to</div>
    <input class="xp-goto" placeholder="/path/to/folder" spellcheck="false">`;
  localCtl.side.querySelectorAll(".xp-place").forEach((n) => (n.onclick = () => localCtl.go({ path: n.dataset.path })));
  localCtl.side.querySelector(".xp-goto").onkeydown = (ev) => {
    if (ev.key === "Enter" && ev.target.value.trim()) localCtl.go({ path: ev.target.value.trim() });
  };
  return r.home;
}

function openEvidence(path) {
  const input = document.getElementById("image-path");
  if (!input) return;
  input.value = path;
  toast(`Analyzing ${path.split("/").pop()} … this can take a few minutes for large images`);
  document.getElementById("btn-analyze")?.click();
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

function viewFor(mime, url, name) {
  if (mime.startsWith("image/")) return `<div class="xp-imgwrap"><img src="${esc(url)}" alt="${esc(name)}"></div>`;
  if (mime === "application/pdf") return `<iframe class="xp-frame" src="${esc(url)}"></iframe>`;
  if (mime.startsWith("video/")) return `<video class="xp-video" controls src="${esc(url)}"></video>`;
  if (mime.startsWith("text/")) return `<iframe class="xp-frame light" src="${esc(url)}"></iframe>`;
  return `<div class="xp-empty">No visual preview for this file type.<br>Use Hex, Text or Strings.</div>`;
}

function guessKind(name) {
  const ext = (name.split(".").pop() || "").toLowerCase();
  if (/^(jpe?g|png|gif|bmp|webp)$/.test(ext)) return "picture";
  if (/^(mp4|mkv|avi|mov|h264|hevc|dav)$/.test(ext)) return "video";
  if (ext === "pdf") return "pdf";
  if (/^(txt|log|csv|json|xml|ini|cfg)$/.test(ext)) return "text";
  if (/^(db|sqlite)$/.test(ext)) return "database";
  if (/^(bin|dat)$/.test(ext)) return "binary data";
  return "file";
}

function copy(text) {
  navigator.clipboard?.writeText(text).then(() => toast("Copied")).catch(() => toast(text));
}

export async function initExplorers() {
  const ir = document.getElementById("xplore-root");
  if (ir) {
    imageCtl = createExplorer(ir, imageSource);
    imageCtl.root = ir;
    refreshImageExplorer();
  }
  const lr = document.getElementById("local-root");
  if (lr) {
    localCtl = createExplorer(lr, localSource);
    try {
      const home = await renderLocalSide();
      localCtl.go({ path: home });
    } catch (e) {
      lr.querySelector(".xp-side").textContent = e.message;
    }
  }
}
