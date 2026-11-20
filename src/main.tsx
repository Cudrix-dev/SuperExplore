import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { AnimatePresence, motion, MotionConfig } from "framer-motion";
import {
  AppWindow, Archive, ArrowDown, ArrowUp, AudioLines, Code2, CornerDownLeft,
  Copy, Database, File, FileText, Folder, FolderOpen, Image, Minus, Search,
  TriangleAlert, Video, X,
} from "lucide-react";
import {
  Fragment, useCallback, useEffect, useMemo, useRef, useState,
  type KeyboardEvent, type MouseEvent, type ReactNode,
} from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

interface Status {
  ready: boolean;
  entries: number;
  volumes: number;
  revision: number;
  warning: string | null;
  shortcutAvailable: boolean;
  materialAvailable: boolean;
}

interface Hit {
  name: string;
  path: string;
  directory: boolean;
  size: number | null;
  modifiedMs: number | null;
}

interface SearchResponse {
  hits: Hit[];
  elapsedMs: number;
}

type ResultAction = "open" | "folder" | "copy";
type ContextMenu = { hit: Hit; x: number; y: number };
const appWindow = getCurrentWindow();
const brand = "/superexplore.svg";
const dateFormat = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric", month: "2-digit", day: "2-digit",
  hour: "2-digit", minute: "2-digit", hourCycle: "h23",
});
const numberFormat = new Intl.NumberFormat("zh-CN");

function Splash() {
  const [elapsed, setElapsed] = useState(false);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState("");
  const [started, setStarted] = useState(false);

  useEffect(() => {
    let stopped = false;
    let deadline: ReturnType<typeof setTimeout>;
    const start = async () => {
      await invoke("show_splash");
      if (stopped) return;
      setStarted(true);
      deadline = setTimeout(() => setElapsed(true), 2000);
    };
    void start().catch((reason) => setError(String(reason)));
    return () => {
      stopped = true;
      clearTimeout(deadline);
    };
  }, []);

  useEffect(() => {
    if (!started || ready) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const check = async () => {
      try {
        const status = await invoke<Status>("app_status");
        if (stopped) return;
        if (status.ready) {
          setReady(true);
          return;
        }
      } catch (reason) {
        if (!stopped) setError(String(reason));
      }
      if (!stopped) timer = setTimeout(() => void check(), 120);
    };
    void check();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [started, ready]);

  const leaving = elapsed && ready;

  useEffect(() => {
    if (!leaving) return;
    const timer = setTimeout(() => {
      void invoke("finish_startup").catch((reason) => {
        setError(String(reason));
        setReady(false);
      });
    }, 125);
    return () => clearTimeout(timer);
  }, [leaving]);

  return (
    <motion.main className="splash" animate={{ opacity: leaving ? 0 : 1 }} transition={{ duration: 0.12 }}>
      <div className="splash-orbit" aria-hidden="true" />
      {!elapsed ? (
        <motion.div className="splash-brand" initial={{ scale: 0.77 }} animate={{ scale: started ? 1 : 0.77 }} transition={{ duration: 1.96, ease: [0.16, 1, 0.3, 1] }}>
          <img src={brand} alt="" width="90" height="90" />
          <h1>SuperExplore</h1>
          <p>Developed by Cudrix</p>
        </motion.div>
      ) : !leaving && (
        <div className="splash-index" role="status">
          <Database size={29} strokeWidth={1.5} />
          <p>正在准备文件索引</p>
          <span className="loader" />
        </div>
      )}
      {error && <p className="splash-error">{error}</p>}
    </motion.main>
  );
}

function useStatus() {
  const [status, setStatus] = useState<Status>();
  useEffect(() => {
    let stopped = false;
    const refresh = async () => {
      try {
        const next = await invoke<Status>("app_status");
        if (!stopped) setStatus(next);
      } catch (error) {
        console.error(error);
      }
    };
    void refresh();
    const timer = setInterval(() => {
      if (!document.hidden) void refresh();
    }, 1500);
    window.addEventListener("focus", refresh);
    return () => {
      stopped = true;
      clearInterval(timer);
      window.removeEventListener("focus", refresh);
    };
  }, []);
  return status;
}

interface Query extends SearchResponse { query: string }

function useSearch(query: string, revision: number) {
  const [result, setResult] = useState<Query>();
  const [error, setError] = useState("");
  const pending = useRef<{ query: string; epoch: number }>();
  const busy = useRef(false);
  const epoch = useRef(0);
  const latestQuery = useRef("");

  const flush = useCallback(async function flushQueue() {
    const task = pending.current;
    if (!task || busy.current) return;
    pending.current = undefined;
    busy.current = true;
    try {
      const response = await invoke<SearchResponse>("search_files", { query: task.query });
      if (epoch.current === task.epoch) {
        setResult({ ...response, query: task.query });
        setError("");
      }
    } catch (reason) {
      if (epoch.current === task.epoch) setError(String(reason));
    } finally {
      busy.current = false;
      void flushQueue();
    }
  }, []);

  useEffect(() => {
    if (latestQuery.current !== query) {
      latestQuery.current = query;
      ++epoch.current;
      setError("");
    }
    pending.current = undefined;
    if (!query || !revision) return;
    const timer = setTimeout(() => {
      pending.current = { query, epoch: epoch.current };
      void flush();
    }, 10);
    return () => clearTimeout(timer);
  }, [query, revision, flush]);

  return { result: result?.query === query ? result : undefined, error };
}

const kinds = [
  { extensions: /^(avif|bmp|gif|heic|ico|jpe?g|png|svg|tiff?|webp)$/, icon: Image, color: "pink" },
  { extensions: /^(aac|flac|m4a|mp3|ogg|wav|wma)$/, icon: AudioLines, color: "pink" },
  { extensions: /^(avi|m4v|mkv|mov|mp4|webm|wmv)$/, icon: Video, color: "pink" },
  { extensions: /^(7z|gz|rar|tar|xz|zip)$/, icon: Archive, color: "amber" },
  { extensions: /^(bat|cmd|exe|lnk|msi)$/, icon: AppWindow, color: "lavender" },
  { extensions: /^(c|cpp|css|html|java|jsx?|json|ps1|py|rs|sql|tsx?|xml|ya?ml)$/, icon: Code2, color: "mint" },
  { extensions: /^(csv|docx?|md|pdf|pptx?|rtf|txt|xlsx?)$/, icon: FileText, color: "blue" },
];

function FileGlyph({ hit }: { hit: Hit }) {
  const kind = hit.directory
    ? { icon: Folder, color: "amber" }
    : kinds.find(({ extensions }) => hit.name.includes(".") && extensions.test(hit.name.split(".").pop()!.toLowerCase()))
      ?? { icon: File, color: "blue" };
  return <span className={"file-glyph " + kind.color}><kind.icon size={16} strokeWidth={1.75} /></span>;
}

function formatSize(bytes: number | null) {
  if (bytes === null) return "—";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let index = 0;
  while (bytes >= 1024 && index < units.length - 1) {
    bytes /= 1024;
    index++;
  }
  return (index === 0 ? String(bytes) : bytes.toFixed(bytes < 10 ? 1 : 0)) + " " + units[index];
}

function queryPattern(query: string) {
  const terms = query.split(/\s+/).sort((a, b) => b.length - a.length)
    .map((term) => term.replace(/[.*+?^$\x7b\x7d()|[\]\\]/g, "\\$&"));
  return new RegExp("(" + terms.join("|") + ")", "giu");
}

function highlight(name: string, pattern: RegExp): ReactNode {
  const pieces = name.split(pattern);
  return pieces.map((part, index) => index % 2
    ? <mark key={index}>{part}</mark>
    : <Fragment key={index}>{part}</Fragment>);
}

function EmptyState({ query, result, error }: { query: string; result?: Query; error: string }) {
  if (error) return <div className="empty-state" role="alert"><TriangleAlert size={26} /><p>{error}</p></div>;
  if (query && !result) return <div className="empty-state" role="status"><span className="loader" /><p>正在搜索…</p></div>;
  return (
    <div className="empty-state" role="status">
      <div className="search-illustration" aria-hidden="true"><Search size={37} strokeWidth={1.25} /><i /></div>
      <h2>{query ? "未找到匹配项" : "输入文件名开始搜索"}</h2>
      <p>{query ? "试试更短的关键词，或检查拼写。" : "用空格分隔多个关键词，缩小搜索范围。"}</p>
    </div>
  );
}

function Main() {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [menu, setMenu] = useState<ContextMenu>();
  const [notice, setNotice] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const context = useRef<HTMLDivElement>(null);
  const status = useStatus();
  const trimmed = query.trim();
  const pattern = useMemo(() => queryPattern(trimmed), [trimmed]);
  const { result, error } = useSearch(trimmed, status?.ready ? status.revision + 1 : 0);
  const hits = result?.hits ?? [];
  const cursor = Math.min(selected, Math.max(hits.length - 1, 0));

  useEffect(() => {
    const focus = () => input.current?.focus({ preventScroll: true });
    window.addEventListener("focus", focus);
    focus();
    return () => window.removeEventListener("focus", focus);
  }, []);

  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(""), 3500);
    return () => clearTimeout(timer);
  }, [notice]);

  useEffect(() => {
    if (!menu) return;
    context.current?.querySelector("button")?.focus();
    const close = (event: PointerEvent) => {
      if (!context.current?.contains(event.target as Node)) setMenu(undefined);
    };
    const blur = () => setMenu(undefined);
    window.addEventListener("pointerdown", close);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("blur", blur);
    };
  }, [menu]);

  const act = async (hit: Hit, action: ResultAction) => {
    setMenu(undefined);
    input.current?.focus({ preventScroll: true });
    try {
      if (action === "copy") {
        await invoke("copy_path", { path: hit.path });
        setNotice("已复制完整路径");
      } else {
        await invoke("open_result", { path: hit.path, containingFolder: action === "folder" });
      }
    } catch (reason) {
      setNotice(String(reason));
    }
  };

  const showMenu = (hit: Hit, x: number, y: number) => {
    setMenu({
      hit,
      x: Math.max(8, Math.min(x, window.innerWidth - 198)),
      y: Math.max(8, Math.min(y, window.innerHeight - 126)),
    });
  };

  const changeQuery = (value: string) => {
    setQuery(value);
    setSelected(0);
    setMenu(undefined);
  };

  const onSearchKeys = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    if (event.key === "Escape") {
      if (query) changeQuery("");
      else void invoke("hide_main");
      event.preventDefault();
      return;
    }
    if (!hits.length) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      const next = (cursor + (event.key === "ArrowDown" ? 1 : -1) + hits.length) % hits.length;
      setSelected(next);
      document.getElementById("hit-" + next)?.scrollIntoView({ block: "nearest" });
    } else if (event.key === "Enter") {
      void act(hits[cursor], event.ctrlKey ? "folder" : "open");
    } else if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
      const row = document.getElementById("hit-" + cursor)?.getBoundingClientRect();
      if (row) showMenu(hits[cursor], row.left + 47, row.bottom);
    } else {
      return;
    }
    event.preventDefault();
  };

  const onMenuKeys = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape" || event.key === "Tab") {
      setMenu(undefined);
      input.current?.focus();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      const buttons = Array.from(event.currentTarget.querySelectorAll("button"));
      const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
      buttons[(current + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length]?.focus();
    } else {
      return;
    }
    event.preventDefault();
  };

  const onDrag = (event: MouseEvent<HTMLElement>) => {
    if (event.button === 0 && !(event.target as HTMLElement).closest("button")) {
      void appWindow.startDragging().catch((reason) => setNotice(String(reason)));
    }
  };

  return (
    <motion.main className={"desktop " + (status?.materialAvailable ? "material" : "")}
      initial={{ opacity: 0.85 }} animate={{ opacity: 1 }} transition={{ duration: 0.18 }}>
      <header className="titlebar flex items-center justify-between" onMouseDown={onDrag}>
        <div className="wordmark flex items-center gap-2">
          <img src={brand} width="23" height="23" alt="" /><span>SuperExplore</span>
        </div>
        <div className="window-actions flex h-full">
          <button aria-label="最小化" title="最小化" onClick={() => void invoke("minimize_main")}><Minus size={16} /></button>
          <button className="close-button" aria-label="关闭到托盘" title="关闭到托盘" onClick={() => void invoke("hide_main")}><X size={16} /></button>
        </div>
      </header>

      <section className="search-area">
        <div className="search-field flex items-center gap-3">
          <Search className="search-icon" size={22} strokeWidth={1.9} />
          <input ref={input} value={query} onChange={(event) => changeQuery(event.target.value)} onKeyDown={onSearchKeys}
            aria-label="搜索文件或文件夹" role="combobox" aria-autocomplete="list" aria-controls="hits"
            aria-expanded={hits.length > 0} aria-activedescendant={hits.length ? "hit-" + cursor : undefined}
            autoComplete="off" spellCheck={false} maxLength={1024} placeholder="搜索文件或文件夹…" />
          <div className="search-end flex items-center gap-3">
            {query && <button className="clear-button" title="清空搜索" aria-label="清空搜索" onClick={() => { changeQuery(""); input.current?.focus(); }}><X size={16} /></button>}
            {status?.shortcutAvailable && <kbd className="shortcut">Alt <span>+</span> Space</kbd>}
          </div>
        </div>
        <div className="search-meta flex items-center justify-between">
          <h1>搜索结果</h1>
          {result && <span>{result.hits.length === 100 ? "前 100 条" : result.hits.length + " 条"} · {result.elapsedMs < 1 ? "< 1" : Math.round(result.elapsedMs)} ms</span>}
        </div>
      </section>

      <section className="result-pane" aria-label="搜索结果">
        <div className="result-grid result-header" aria-hidden="true">
          <span>名称</span><span>完整路径</span><span>大小</span><span>修改日期</span>
        </div>
        <div className="results" id="hits" role="listbox" aria-label="搜索结果">
          {hits.map((hit, index) => (
            <div id={"hit-" + index} role="option" aria-selected={cursor === index} key={hit.path}
              className={"result-grid result-row " + (cursor === index ? "selected" : "")}
              onMouseDown={(event) => { if (event.button === 0) event.preventDefault(); }}
              onClick={() => { setSelected(index); input.current?.focus(); }}
              onDoubleClick={() => void act(hit, "open")}
              onContextMenu={(event) => { event.preventDefault(); setSelected(index); showMenu(hit, event.clientX, event.clientY); }}>
              <span className="result-name" title={hit.name}><FileGlyph hit={hit} /><b>{highlight(hit.name, pattern)}</b></span>
              <span className="result-path" title={hit.path} dir="ltr">{hit.path}</span>
              <span>{formatSize(hit.size)}</span>
              <span>{hit.modifiedMs === null ? "—" : dateFormat.format(hit.modifiedMs)}</span>
            </div>
          ))}
        </div>
        {!hits.length && <EmptyState query={trimmed} result={result} error={error} />}
      </section>

      <footer className="statusbar flex items-center justify-between gap-3">
        <div className="status-info flex items-center gap-2" title={status?.warning ?? (status?.shortcutAvailable === false ? "Alt + Space 已被其他程序占用，可从托盘打开。" : undefined)}>
          {status?.warning || status?.shortcutAvailable === false ? <TriangleAlert size={14} className="warning-icon" /> : <span className="status-dot" />}
          <span>{status?.ready ? "已索引 " + numberFormat.format(status.entries) + " 个项目 · " + status.volumes + " 个卷" : "正在读取索引…"}</span>
          {status?.shortcutAvailable === false && <span className="shortcut-warning">· 快捷键不可用</span>}
        </div>
        <div className="key-guide flex items-center gap-4">
          <span><kbd><ArrowUp size={11} /><ArrowDown size={11} /></kbd> 选择</span>
          <span><kbd><CornerDownLeft size={11} /></kbd> 打开</span>
          <span><kbd>Ctrl <CornerDownLeft size={11} /></kbd> 所在目录</span>
        </div>
      </footer>

      {menu && (
        <div ref={context} role="menu" className="context-menu" style={{ left: menu.x, top: menu.y }} onKeyDown={onMenuKeys}>
          <button role="menuitem" onClick={() => void act(menu.hit, "open")}><AppWindow size={15} />打开</button>
          <button role="menuitem" onClick={() => void act(menu.hit, "folder")}><FolderOpen size={15} />打开所在目录</button>
          <button role="menuitem" onClick={() => void act(menu.hit, "copy")}><Copy size={15} />复制完整路径</button>
        </div>
      )}

      <AnimatePresence>
        {notice && <motion.div role="status" className="toast" initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: 6 }}>{notice}</motion.div>}
      </AnimatePresence>
    </motion.main>
  );
}

createRoot(document.getElementById("root")!).render(
  <MotionConfig reducedMotion="user">{appWindow.label === "splash" ? <Splash /> : <Main />}</MotionConfig>,
);
