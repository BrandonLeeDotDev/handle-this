const vscode = require('vscode');
const path = require('path');
const fs = require('fs');
const { spawn, execSync } = require('child_process');

let lspProcess = null;
let lspDiagnostics = null;
let requestId = 1;
let pendingRequests = new Map();

// Keyword documentation for hover
const KEYWORD_DOCS = {
    'try': {
        title: 'try',
        description: 'Execute a block and wrap any error with automatic stack trace capture.',
        syntax: '`try { expr }` or `try { expr } with "context"`',
        example: `handle! {
    try { std::fs::read_to_string(path)? }
    with "reading config"
}`
    },
    'catch': {
        title: 'catch',
        description: 'Recover from errors by converting them to success values. Can match specific error types.',
        syntax: '`catch e { recovery }` or `catch Type(e) { recovery }` or `catch Type(e) when guard { recovery }`',
        example: `handle! {
    try { might_fail()? }
    catch io::Error(e) when e.kind() == NotFound {
        default_value
    }
}`
    },
    'throw': {
        title: 'throw',
        description: 'Map errors to a different error type. Use with catch to inspect before remapping.',
        syntax: '`throw e { new_error }` or `catch e { log(e); } throw { "sanitized" }`',
        example: `handle! {
    try { internal_op()? }
    catch e { log::error!("{e}"); }
    throw { "service unavailable" }
    with "api call"
}`
    },
    'finally': {
        title: 'finally',
        description: 'Cleanup code that runs regardless of success or failure.',
        syntax: '`try { expr } finally { cleanup }`',
        example: `handle! {
    try { use_resource()? }
    finally { release_resource(); }
}`
    },
    'with': {
        title: 'with',
        description: 'Add context message and/or structured key-value data to errors.',
        syntax: '`with "message"` or `with { "key": value }` or `with "message", { "key": value }`',
        example: `handle! {
    try { fetch(url)? }
    with "fetching data", { "url": url, "retry": attempt }
}`
    },
    'then': {
        title: 'then',
        description: 'Chain operations with closure syntax. Terminal catch/throw handles errors from any step.',
        syntax: '`then |x| { transform(x) }`',
        example: `handle! {
    try { read_file(path)? } with "reading",
    then |s| { parse(s)? } with "parsing",
    then |data| { validate(data)? }
        catch { default_data() }
}`
    },
    'require': {
        title: 'require',
        description: 'Assert a condition at the start of a handle block. Returns error if condition fails.',
        syntax: '`require condition else "error message"`',
        example: `handle! {
    require !path.is_empty() else "path required",
    try { read(path)? }
}`
    },
    'else': {
        title: 'else',
        description: 'Used in two contexts:\n- **require:** Specifies error value when condition fails\n- **try when:** Alternative branch when condition is false',
        syntax: '`require condition else error_expr` or `try when cond { } else { }`',
        example: `// With require
require user.is_authenticated() else "unauthorized"

// With try when
try when n > 0 { positive() } else { negative() }`
    },
    'else when': {
        title: 'else when',
        description: 'Additional conditional branch in a try when chain. Evaluated if previous conditions were false.',
        syntax: '`try when cond1 { } else when cond2 { } else { }`',
        example: `handle! {
    try when x > 10 { "large" }
    else when x > 0 { "small" }
    else { "zero or negative" }
}`
    },
    'any': {
        title: 'any',
        description: 'Try each item in a collection until one succeeds. Returns first success or last error.',
        syntax: '`try any item in collection { expr }`',
        example: `handle! {
    try any server in &servers { connect(server)? }
    catch { fallback_connection() }
}`
    },
    'all': {
        title: 'all',
        description: 'Try all items in a collection. Fails on first error. Returns Vec of results on success.',
        syntax: '`try all item in collection { expr }`',
        example: `handle! {
    try all file in &files { validate(file)? }
    with "validating batch"
}`
    },
    'when': {
        title: 'when',
        description: 'Guard condition for typed catch. Only catches if type matches AND guard is true.',
        syntax: '`catch Type(e) when guard_expr { recovery }`',
        example: `catch io::Error(e) when e.kind() == ErrorKind::NotFound {
    create_default()
}`
    },
    'async': {
        title: 'async',
        description: 'Mark a try block as async for use with .await operations.',
        syntax: '`async try { expr.await }`',
        example: `handle! {
    async try { fetch_data(url).await? }
    with "async fetch"
}`
    },
    'scope': {
        title: 'scope',
        description: 'Add a named scope with file/line info and context to errors.',
        syntax: '`.scope(file, line, col, "context")`',
        example: `error.scope(file!(), line!(), column!(), "in parser")`
    },
    'inspect': {
        title: 'inspect',
        description: 'Examine and optionally modify an error without consuming it. Used in error pipelines.',
        syntax: '`.inspect(|e| { log(e); })`',
        example: `result.inspect_err(|e| log::warn!("{e}"))`
    },
    'while': {
        title: 'while',
        description: 'Retry an operation while a condition is true or until success.',
        syntax: '`try while condition { expr }`',
        example: `handle! {
    try while attempts < 3 { connect()? }
    with "connecting with retry"
}`
    },
    // Multi-word keyword combinations
    'try when': {
        title: 'try when',
        description: 'Execute a block only when a condition is true. Skips execution if condition is false.',
        syntax: '`try when condition { expr }`',
        example: `handle! {
    try when should_fetch { fetch_data()? }
    with "conditional fetch"
}`
    },
    'try any': {
        title: 'try any',
        description: 'Try each item in a collection until one succeeds. Returns first success or last error.',
        syntax: '`try any item in collection { expr }`',
        example: `handle! {
    try any server in &servers { connect(server)? }
    catch { fallback_connection() }
}`
    },
    'try all': {
        title: 'try all',
        description: 'Try all items in a collection. Fails on first error. Returns Vec of results on success.',
        syntax: '`try all item in collection { expr }`',
        example: `handle! {
    try all file in &files { validate(file)? }
    with "validating batch"
}`
    },
    'try while': {
        title: 'try while',
        description: 'Retry an operation while a condition is true or until success.',
        syntax: '`try while condition { expr }`',
        example: `handle! {
    try while attempts < 3 { connect()? }
    with "connecting with retry"
}`
    },
    'async try': {
        title: 'async try',
        description: 'Execute an async block with .await operations and automatic error wrapping.',
        syntax: '`async try { expr.await }`',
        example: `handle! {
    async try { fetch_data(url).await? }
    with "async fetch"
}`
    },
    'catch any': {
        title: 'catch any',
        description: 'Catch errors from any item in a try any block individually.',
        syntax: '`catch any Type(e) { recovery }`',
        example: `handle! {
    try any server in &servers { connect(server)? }
    catch any io::Error(e) { fallback() }
}`
    },
    'catch all': {
        title: 'catch all',
        description: 'Catch errors from all items in a try all block.',
        syntax: '`catch all Type(e) { recovery }`',
        example: `handle! {
    try all file in &files { process(file)? }
    catch all io::Error(_) { skip() }
}`
    },
    'try for': {
        title: 'try for',
        description: 'Iterate over a collection, executing the block for each item. Collects results or fails on first error.',
        syntax: '`try for item in collection { expr }`',
        example: `handle! {
    try for file in &files { process(file)? }
    with "processing files"
}`
    },
    'throw any': {
        title: 'throw any',
        description: 'Remap errors from a try any block to a different error type.',
        syntax: '`throw any Type(e) { new_error }`',
        example: `handle! {
    try any server in &servers { connect(server)? }
    throw any io::Error(e) { format!("connection failed: {}", e) }
}`
    },
    'throw all': {
        title: 'throw all',
        description: 'Remap errors from a try all block to a different error type.',
        syntax: '`throw all Type(e) { new_error }`',
        example: `handle! {
    try all file in &files { validate(file)? }
    throw all io::Error(e) { format!("validation failed: {}", e) }
}`
    },
    'inspect any': {
        title: 'inspect any',
        description: 'Inspect errors from a try any block without consuming them. Useful for logging.',
        syntax: '`inspect any Type(e) { log(e); }`',
        example: `handle! {
    try any server in &servers { connect(server)? }
    inspect any io::Error(e) { log::warn!("attempt failed: {}", e); }
    catch { fallback() }
}`
    },
    'inspect all': {
        title: 'inspect all',
        description: 'Inspect errors from a try all block without consuming them. Useful for logging.',
        syntax: '`inspect all Type(e) { log(e); }`',
        example: `handle! {
    try all file in &files { process(file)? }
    inspect all io::Error(e) { log::warn!("file error: {}", e); }
    throw { "batch failed" }
}`
    }
};

const BUILTIN_THEMES = {
    'rainbow':     { keyword: '#C586C0', modifier: '#C586C0', async: '#4FC1FF', string: '#CE9178', number: '#B5CEA8', comment: '#6A9955', function: '#DCDCAA', type: '#4EC9B0', variable: '#9CDCFE', macro: '#569CD6', operator: '#D4D4D4', attribute: '#9CDCFE', punctuation: '#FFD700' },
    'dark-plus':   { keyword: '#C586C0', modifier: '#56B6C2', async: '#4FC1FF', string: '#CE9178', number: '#B5CEA8', comment: '#6A9955', function: '#DCDCAA', type: '#4EC9B0', variable: '#9CDCFE', macro: '#569CD6', operator: '#D4D4D4', attribute: '#9CDCFE', punctuation: '#D4D4D4' },
    'monokai':     { keyword: '#F92672', modifier: '#A6E22E', async: '#66D9EF', string: '#E6DB74', number: '#AE81FF', comment: '#75715E', function: '#A6E22E', type: '#66D9EF', variable: '#F8F8F2', macro: '#66D9EF', operator: '#F92672', attribute: '#A6E22E', punctuation: '#F8F8F2' },
    'dracula':     { keyword: '#FF79C6', modifier: '#8BE9FD', async: '#8BE9FD', string: '#F1FA8C', number: '#BD93F9', comment: '#6272A4', function: '#50FA7B', type: '#8BE9FD', variable: '#F8F8F2', macro: '#8BE9FD', operator: '#FF79C6', attribute: '#50FA7B', punctuation: '#F8F8F2' },
    'one-dark':    { keyword: '#C678DD', modifier: '#56B6C2', async: '#61AFEF', string: '#98C379', number: '#D19A66', comment: '#5C6370', function: '#61AFEF', type: '#E5C07B', variable: '#E06C75', macro: '#61AFEF', operator: '#56B6C2', attribute: '#E5C07B', punctuation: '#ABB2BF' },
    'nord':        { keyword: '#B48EAD', modifier: '#88C0D0', async: '#81A1C1', string: '#A3BE8C', number: '#B48EAD', comment: '#616E88', function: '#88C0D0', type: '#8FBCBB', variable: '#D8DEE9', macro: '#81A1C1', operator: '#81A1C1', attribute: '#8FBCBB', punctuation: '#ECEFF4' },
    'gruvbox':     { keyword: '#FB4934', modifier: '#8EC07C', async: '#83A598', string: '#B8BB26', number: '#D3869B', comment: '#928374', function: '#FABD2F', type: '#8EC07C', variable: '#EBDBB2', macro: '#83A598', operator: '#FE8019', attribute: '#FABD2F', punctuation: '#EBDBB2' },
    'catppuccin':  { keyword: '#F5C2E7', modifier: '#94E2D5', async: '#89B4FA', string: '#A6E3A1', number: '#FAB387', comment: '#6C7086', function: '#89DCEB', type: '#94E2D5', variable: '#CDD6F4', macro: '#89B4FA', operator: '#89DCEB', attribute: '#F9E2AF', punctuation: '#CDD6F4' }
};

const RAINBOW_COLORS = {
    'handle-this.h.rust': '#FF6B6B', 'handle-this.a.rust': '#FFA94D', 'handle-this.n.rust': '#FFE066',
    'handle-this.d.rust': '#69DB7C', 'handle-this.l.rust': '#4DABF7', 'handle-this.e.rust': '#9775FA',
    'handle-this.bang.rust': '#F783AC'
};

const HANDLE_SCOPES = ['keyword.control.handle-this.rust', 'keyword.modifier.handle-this.rust', 'keyword.async.handle-this.rust',
    'entity.name.type.handler.handle-this.rust', 'variable.parameter.handle-this.rust', 'punctuation.brackets.handle-this.rust',
    'handle-this.h.rust', 'handle-this.a.rust', 'handle-this.n.rust', 'handle-this.d.rust',
    'handle-this.l.rust', 'handle-this.e.rust', 'handle-this.bang.rust'];

const RUST_SCOPES = ['string.quoted.double.rust', 'string.quoted.single.rust', 'constant.numeric',
    'comment.line.double-slash.rust', 'comment.block.rust', 'comment.block.documentation.rust',
    'entity.name.function.rust', 'entity.name.type', 'support.type', 'variable.other.rust',
    'entity.name.function.macro.rust', 'support.macro.rust', 'keyword.operator',
    'meta.attribute.rust', 'punctuation.brackets.attribute.rust', 'punctuation.definition.attribute.rust'];

const ALL_SCOPES = [...HANDLE_SCOPES, ...RUST_SCOPES];
const COLOR_FIELDS = ['keyword','modifier','async','string','number','comment','function','type','variable','macro','operator','attribute','punctuation'];

let settings = { preset: 'rainbow', rainbow: true, macroColor: '#DCDCAA', customThemes: {} };
let ctx = null;
let panel = null;

function load() { 
    if (!ctx) return;
    const s = ctx.globalState.get('handleThisSettings');
    if (s) settings = { ...settings, ...s };
}
function save() { if (ctx) ctx.globalState.update('handleThisSettings', settings); }

function getActiveColors() {
    const base = BUILTIN_THEMES[settings.preset] || BUILTIN_THEMES['rainbow'];
    const custom = settings.customThemes[settings.preset];
    // Merge custom over base to ensure all fields exist (handles upgrades with new fields)
    return custom ? { ...base, ...custom } : base;
}

function apply() {
    load();
    const c = getActiveColors();
    const rules = [];
    
    rules.push({ scope: 'keyword.control.handle-this.rust', settings: { foreground: c.keyword } });
    rules.push({ scope: 'keyword.modifier.handle-this.rust', settings: { foreground: c.modifier } });
    rules.push({ scope: 'keyword.async.handle-this.rust', settings: { foreground: c.async } });
    rules.push({ scope: 'entity.name.type.handler.handle-this.rust', settings: { foreground: c.type } });
    rules.push({ scope: 'variable.parameter.handle-this.rust', settings: { foreground: c.variable } });
    rules.push({ scope: 'punctuation.brackets.handle-this.rust', settings: { foreground: c.punctuation } });

    if (settings.rainbow) {
        for (const [scope, color] of Object.entries(RAINBOW_COLORS)) rules.push({ scope, settings: { foreground: color } });
    } else {
        for (const scope of Object.keys(RAINBOW_COLORS)) rules.push({ scope, settings: { foreground: settings.macroColor } });
    }
    
    rules.push({ scope: 'string.quoted.double.rust', settings: { foreground: c.string } });
    rules.push({ scope: 'string.quoted.single.rust', settings: { foreground: c.string } });
    rules.push({ scope: 'constant.numeric', settings: { foreground: c.number } });
    rules.push({ scope: 'comment.line.double-slash.rust', settings: { foreground: c.comment } });
    rules.push({ scope: 'comment.block.rust', settings: { foreground: c.comment } });
    rules.push({ scope: 'comment.block.documentation.rust', settings: { foreground: c.comment } });
    rules.push({ scope: 'entity.name.function.rust', settings: { foreground: c.function } });
    rules.push({ scope: 'entity.name.type', settings: { foreground: c.type } });
    rules.push({ scope: 'support.type', settings: { foreground: c.type } });
    rules.push({ scope: 'variable.other.rust', settings: { foreground: c.variable } });
    rules.push({ scope: 'entity.name.function.macro.rust', settings: { foreground: c.macro } });
    rules.push({ scope: 'support.macro.rust', settings: { foreground: c.macro } });
    rules.push({ scope: 'keyword.operator', settings: { foreground: c.operator } });
    rules.push({ scope: 'meta.attribute.rust', settings: { foreground: c.attribute } });
    rules.push({ scope: 'punctuation.brackets.attribute.rust', settings: { foreground: c.punctuation } });
    rules.push({ scope: 'punctuation.definition.attribute.rust', settings: { foreground: c.punctuation } });

    const cfg = vscode.workspace.getConfiguration('editor');
    const cur = cfg.get('tokenColorCustomizations') || {};
    const existing = (cur.textMateRules || []).filter(r => r && r.scope && !ALL_SCOPES.includes(r.scope));
    cfg.update('tokenColorCustomizations', { ...cur, textMateRules: [...existing, ...rules] }, vscode.ConfigurationTarget.Global);
}

function webview() {
    load();
    const c = getActiveColors();
    
    const pureCustom = Object.keys(settings.customThemes).filter(k => !BUILTIN_THEMES[k]);
    const allNames = [...Object.keys(BUILTIN_THEMES), ...pureCustom];
    const opts = allNames.map(n => `<option value="${n}" ${settings.preset===n?'selected':''}>${n}</option>`).join('');
    
    return `<!DOCTYPE html><html><head><style>
        body{font-family:var(--vscode-font-family);padding:20px;color:var(--vscode-foreground);background:var(--vscode-editor-background)}
        h2{margin-top:0;border-bottom:1px solid var(--vscode-panel-border);padding-bottom:10px}
        h3{margin-top:20px;margin-bottom:10px;color:var(--vscode-descriptionForeground);font-size:12px;text-transform:uppercase}
        .row{display:flex;align-items:center;margin:6px 0}
        label{width:100px;font-size:12px}
        input[type="color"]{width:40px;height:26px;border:none;cursor:pointer;background:none;padding:0}
        input[type="color"]::-webkit-color-swatch-wrapper{padding:0}
        input[type="color"]::-webkit-color-swatch{border:1px solid #555;border-radius:3px}
        .hex{margin-left:6px;font-family:monospace;font-size:11px;color:var(--vscode-descriptionForeground);width:60px}
        select{padding:4px 8px;background:var(--vscode-dropdown-background);color:var(--vscode-dropdown-foreground);border:1px solid var(--vscode-dropdown-border);border-radius:3px;min-width:150px}
        input[type="text"]{padding:4px 8px;background:var(--vscode-input-background);color:var(--vscode-input-foreground);border:1px solid var(--vscode-input-border);border-radius:3px;width:150px}
        .checkbox-row{display:flex;align-items:center;gap:6px;margin:8px 0}
        input[type="checkbox"]{width:14px;height:14px;cursor:pointer}
        .preview{background:#1e1e1e;padding:12px;border-radius:4px;font-family:'Consolas','Monaco',monospace;font-size:12px;margin-top:15px;line-height:1.4}
        .preview-label{color:#888;margin-bottom:6px;font-size:10px;text-transform:uppercase}
        button{background:var(--vscode-button-background);color:var(--vscode-button-foreground);border:none;padding:6px 14px;border-radius:3px;cursor:pointer;font-size:12px;margin-right:8px}
        button:hover{background:var(--vscode-button-hoverBackground)}
        .btn-secondary{background:var(--vscode-button-secondaryBackground);color:var(--vscode-button-secondaryForeground)}
        .buttons{display:flex;margin-top:15px;flex-wrap:wrap;gap:8px}
        .columns{display:flex;gap:30px}.column{flex:1}
        .theme-row{display:flex;align-items:center;gap:10px;margin-bottom:10px}
        .delete-theme{color:#f44;cursor:pointer;font-size:14px;margin-left:5px}
        .modified{color:#FFA500;font-size:11px;margin-left:10px;font-weight:bold}
        .save-row{display:flex;align-items:center;gap:8px;margin-top:10px}
        .save-row.hidden{display:none}
    </style></head><body>
    <h2>handle-this Colors</h2>
    <div class="theme-row">
        <label>Theme:</label>
        <select id="preset">${opts}</select>
        <span id="deleteTheme" class="delete-theme" style="display:none">✕</span>
        <span id="modified" class="modified" style="display:none">● Modified</span>
    </div>
    <div class="checkbox-row">
        <input type="checkbox" id="rainbow" ${settings.rainbow?'checked':''}>
        <label for="rainbow" style="width:auto">Rainbow handle!</label>
    </div>
    <div class="columns">
        <div class="column">
            <h3>handle-this</h3>
            <div class="row"><label>Keywords:</label><input type="color" id="keyword" value="${c.keyword}"><span class="hex">${c.keyword}</span></div>
            <div class="row"><label>Modifiers:</label><input type="color" id="modifier" value="${c.modifier}"><span class="hex">${c.modifier}</span></div>
            <div class="row"><label>async:</label><input type="color" id="async" value="${c.async}"><span class="hex">${c.async}</span></div>
            <div class="row"><label>handle!:</label><input type="color" id="macroColor" value="${settings.macroColor}"><span class="hex">${settings.macroColor}</span></div>
            <h3>Syntax</h3>
            <div class="row"><label>Strings:</label><input type="color" id="string" value="${c.string}"><span class="hex">${c.string}</span></div>
            <div class="row"><label>Numbers:</label><input type="color" id="number" value="${c.number}"><span class="hex">${c.number}</span></div>
            <div class="row"><label>Comments:</label><input type="color" id="comment" value="${c.comment}"><span class="hex">${c.comment}</span></div>
        </div>
        <div class="column">
            <h3>Identifiers</h3>
            <div class="row"><label>Functions:</label><input type="color" id="function" value="${c.function}"><span class="hex">${c.function}</span></div>
            <div class="row"><label>Types:</label><input type="color" id="type" value="${c.type}"><span class="hex">${c.type}</span></div>
            <div class="row"><label>Variables:</label><input type="color" id="variable" value="${c.variable}"><span class="hex">${c.variable}</span></div>
            <div class="row"><label>Macros:</label><input type="color" id="macro" value="${c.macro}"><span class="hex">${c.macro}</span></div>
            <div class="row"><label>Operators:</label><input type="color" id="operator" value="${c.operator}"><span class="hex">${c.operator}</span></div>
            <h3>Attributes</h3>
            <div class="row"><label>Attributes:</label><input type="color" id="attribute" value="${c.attribute}"><span class="hex">${c.attribute}</span></div>
            <div class="row"><label>#[] brackets:</label><input type="color" id="punctuation" value="${c.punctuation}"><span class="hex">${c.punctuation}</span></div>
        </div>
    </div>
    <div class="preview-label">Preview:</div>
    <div class="preview">
        <span id="pComment">// Fetch and process</span><br>
        <span style="color:#569CD6">let</span> <span id="pVar1">result</span> <span style="color:#D4D4D4">=</span> <span id="pHandle">handle!</span> <span style="color:#D4D4D4">{</span><br>
        &nbsp;&nbsp;<span id="pRequire">require</span> <span style="color:#D4D4D4">!</span><span id="pVar6">url</span><span style="color:#D4D4D4">.</span><span id="pFn3">is_empty</span><span style="color:#D4D4D4">()</span> <span id="pElse">else</span> <span id="pStr3">"url required"</span><span style="color:#D4D4D4">,</span><br>
        &nbsp;&nbsp;<span id="pTry">try</span> <span id="pAny">any</span> <span id="pVar7">server</span> <span style="color:#569CD6">in</span> <span style="color:#D4D4D4">&amp;</span><span id="pVar9">servers</span> <span style="color:#D4D4D4">{</span> <span id="pFn">connect</span><span style="color:#D4D4D4">(</span><span id="pVar10">server</span><span style="color:#D4D4D4">)?</span> <span style="color:#D4D4D4">}</span><br>
        &nbsp;&nbsp;&nbsp;&nbsp;<span id="pCatch">catch</span> <span id="pAny2">any</span> <span id="pType3">io::Error</span><span style="color:#D4D4D4">(</span><span id="pVar8">e</span><span style="color:#D4D4D4">) {</span> <span id="pStr1">"fallback"</span> <span style="color:#D4D4D4">},</span><br>
        &nbsp;&nbsp;<span id="pTry2">try</span> <span id="pAll">all</span> <span id="pVar11">item</span> <span style="color:#569CD6">in</span> <span style="color:#D4D4D4">&amp;</span><span id="pVar12">items</span> <span style="color:#D4D4D4">{</span> <span id="pFn2">validate</span><span style="color:#D4D4D4">(</span><span id="pVar13">item</span><span style="color:#D4D4D4">)?</span> <span style="color:#D4D4D4">}</span><br>
        <span style="color:#D4D4D4">};</span>
    </div>
    
    <div class="buttons">
        <button id="apply">Apply</button>
        <button id="reset" class="btn-secondary">Reset to Default</button>
    </div>
    
    <div class="save-row hidden" id="saveRow">
        <input type="text" id="newThemeName" placeholder="New theme name">
        <button id="saveTheme">Save Theme</button>
        <button id="cancelSave" class="btn-secondary">Cancel</button>
    </div>
    <div class="buttons">
        <button id="showSave" class="btn-secondary">Save as New Theme...</button>
    </div>

    <script>
        const vscode = acquireVsCodeApi();
        const builtin = ${JSON.stringify(BUILTIN_THEMES)};
        const fields = ${JSON.stringify(COLOR_FIELDS)};
        const rainbow = ['#FF6B6B','#FFA94D','#FFE066','#69DB7C','#4DABF7','#9775FA','#F783AC'];
        
        // Load state from extension (passed fresh each time)
        let custom = ${JSON.stringify(settings.customThemes)};
        let currentPreset = '${settings.preset}';

        function getBuiltinDefault(name) { return builtin[name] || null; }
        function getSaved(name) {
            const base = builtin[name] || builtin['rainbow'];
            return custom[name] ? { ...base, ...custom[name] } : base;
        }
        function isPureCustom(name) { return !builtin[name] && custom[name]; }
        
        function getUI() {
            const c = {};
            fields.forEach(f => c[f] = document.getElementById(f).value);
            return c;
        }
        
        function setUI(c) {
            fields.forEach(f => {
                const el = document.getElementById(f);
                if (el && c[f]) { el.value = c[f]; el.nextElementSibling.textContent = c[f]; }
            });
        }
        
        function colorsEqual(a, b) {
            return fields.every(f => (a[f]||'').toUpperCase() === (b[f]||'').toUpperCase());
        }
        
        function checkModified() {
            const name = document.getElementById('preset').value;
            const def = getBuiltinDefault(name);
            if (!def) {
                document.getElementById('modified').style.display = 'none';
                return;
            }
            const ui = getUI();
            const mod = !colorsEqual(def, ui);
            document.getElementById('modified').style.display = mod ? 'inline' : 'none';
        }
        
        function updateDeleteButton() {
            const name = document.getElementById('preset').value;
            document.getElementById('deleteTheme').style.display = isPureCustom(name) ? 'inline' : 'none';
        }
        
        function loadTheme(name) {
            setUI(getSaved(name));
            updateDeleteButton();
            checkModified();
            updatePreview();
        }
        
        function updatePreview() {
            const c = getUI();
            const isRainbow = document.getElementById('rainbow').checked;
            ['pTry','pTry2','pCatch','pRequire','pElse'].forEach(id => { const el = document.getElementById(id); if(el) el.style.color = c.keyword; });
            ['pAny','pAny2','pAll'].forEach(id => { const el = document.getElementById(id); if(el) el.style.color = c.modifier; });
            ['pStr1','pStr3'].forEach(id => { const el = document.getElementById(id); if(el) el.style.color = c.string; });
            document.getElementById('pComment').style.color = c.comment;
            ['pFn','pFn2','pFn3'].forEach(id => { const el = document.getElementById(id); if(el) el.style.color = c.function; });
            ['pType3'].forEach(id => { const el = document.getElementById(id); if(el) el.style.color = c.type; });
            ['pVar1','pVar6','pVar7','pVar8','pVar9','pVar10','pVar11','pVar12','pVar13'].forEach(id => { const el = document.getElementById(id); if(el) el.style.color = c.variable; });
            const h = document.getElementById('pHandle');
            h.innerHTML = isRainbow ? 'handle!'.split('').map((x,i)=>'<span style="color:'+rainbow[i]+'">'+x+'</span>').join('') : '<span style="color:'+document.getElementById('macroColor').value+'">handle!</span>';
        }
        
        document.getElementById('preset').addEventListener('change', e => loadTheme(e.target.value));
        document.getElementById('rainbow').addEventListener('change', updatePreview);
        [...fields,'macroColor'].forEach(f => {
            const el = document.getElementById(f);
            if(el) el.addEventListener('input', () => { el.nextElementSibling.textContent = el.value; checkModified(); updatePreview(); });
        });
        
        document.getElementById('apply').addEventListener('click', () => {
            const name = document.getElementById('preset').value;
            const colors = getUI();
            custom[name] = colors;
            vscode.postMessage({ cmd:'apply', preset:name, rainbow:document.getElementById('rainbow').checked, macroColor:document.getElementById('macroColor').value, custom });
            checkModified();
        });
        
        document.getElementById('reset').addEventListener('click', () => {
            const name = document.getElementById('preset').value;
            const def = getBuiltinDefault(name);
            if (def) {
                setUI(def);
                delete custom[name];
                checkModified();
                updatePreview();
                vscode.postMessage({ cmd:'reset', preset:name, custom });
            }
        });
        
        document.getElementById('showSave').addEventListener('click', () => {
            document.getElementById('saveRow').classList.remove('hidden');
            document.getElementById('newThemeName').focus();
        });
        
        document.getElementById('cancelSave').addEventListener('click', () => {
            document.getElementById('saveRow').classList.add('hidden');
            document.getElementById('newThemeName').value = '';
        });
        
        document.getElementById('saveTheme').addEventListener('click', () => {
            const name = document.getElementById('newThemeName').value.trim();
            if (!name) { return; }
            if (builtin[name]) { vscode.postMessage({ cmd:'error', msg:'Cannot use built-in theme name' }); return; }
            
            custom[name] = getUI();
            const sel = document.getElementById('preset');
            if (!Array.from(sel.options).some(o=>o.value===name)) {
                const opt = document.createElement('option'); opt.value=name; opt.textContent=name; sel.appendChild(opt);
            }
            sel.value = name;
            updateDeleteButton();
            checkModified();
            document.getElementById('saveRow').classList.add('hidden');
            document.getElementById('newThemeName').value = '';
            vscode.postMessage({ cmd:'saveNew', name, custom });
        });
        
        document.getElementById('deleteTheme').addEventListener('click', () => {
            const name = document.getElementById('preset').value;
            if (builtin[name]) return;
            delete custom[name];
            const sel = document.getElementById('preset');
            const opt = sel.querySelector('option[value="'+name+'"]');
            if (opt) opt.remove();
            sel.value = 'rainbow';
            loadTheme('rainbow');
            vscode.postMessage({ cmd:'delete', name, custom });
        });
        
        updateDeleteButton();
        checkModified();
        updatePreview();
    </script></body></html>`;
}

function openPicker() {
    if (panel) {
        panel.reveal();
        panel.webview.html = webview(); // Refresh with current state
        return;
    }
    
    panel = vscode.window.createWebviewPanel(
        'handleThisColors', 
        'handle-this Colors', 
        vscode.ViewColumn.One, 
        { 
            enableScripts: true,
            retainContextWhenHidden: true // Keep webview alive when hidden
        }
    );
    
    panel.webview.html = webview();
    
    panel.onDidDispose(() => { panel = null; }, null, ctx.subscriptions);
    
    panel.webview.onDidReceiveMessage(m => {
        if (m.cmd === 'apply') {
            settings.preset = m.preset;
            settings.rainbow = m.rainbow;
            settings.macroColor = m.macroColor;
            settings.customThemes = m.custom;
            save(); apply();
            vscode.window.showInformationMessage('Colors applied!');
        } else if (m.cmd === 'reset') {
            settings.preset = m.preset;
            settings.customThemes = m.custom;
            save(); apply();
            vscode.window.showInformationMessage('Reset to default!');
        } else if (m.cmd === 'saveNew') {
            settings.customThemes = m.custom;
            settings.preset = m.name;
            save(); apply();
            vscode.window.showInformationMessage('Theme "'+m.name+'" saved!');
        } else if (m.cmd === 'delete') {
            settings.customThemes = m.custom;
            settings.preset = 'rainbow';
            save(); apply();
            vscode.window.showInformationMessage('Theme deleted!');
        } else if (m.cmd === 'error') {
            vscode.window.showErrorMessage(m.msg);
        }
    }, undefined, ctx.subscriptions);
}

// Check if position is inside a handle! macro invocation
function isInsideHandleMacro(document, position) {
    const text = document.getText();
    const offset = document.offsetAt(position);

    // Search backwards for handle! or handled!
    let depth = 0;
    let i = offset;
    while (i > 0) {
        i--;
        const char = text[i];
        if (char === '}') depth++;
        else if (char === '{') {
            if (depth > 0) depth--;
            else {
                // Found unmatched opening brace, check if preceded by handle! or handled!
                const before = text.substring(Math.max(0, i - 20), i).trimEnd();
                if (before.endsWith('handle!') || before.endsWith('handled!')) {
                    return true;
                }
            }
        }
    }
    return false;
}

// Multi-word keyword patterns to check
const MULTI_WORD_PATTERNS = [
    'try when', 'try any', 'try all', 'try while', 'try for',
    'async try', 'catch any', 'catch all', 'throw any', 'throw all',
    'inspect any', 'inspect all', 'else when'
];

// Get the keyword at the current position, checking for multi-word patterns first
function getKeywordAtPosition(document, position) {
    const line = document.lineAt(position.line).text;
    const wordRange = document.getWordRangeAtPosition(position, /\b[a-z_]+\b/);
    if (!wordRange) return null;

    const word = document.getText(wordRange);
    const wordStart = wordRange.start.character;
    const wordEnd = wordRange.end.character;

    // Check for multi-word patterns
    for (const pattern of MULTI_WORD_PATTERNS) {
        const [first, second] = pattern.split(' ');

        // Check if current word is first part and next word is second part
        if (word === first) {
            const afterWord = line.substring(wordEnd).match(/^\s+([a-z_]+)/);
            if (afterWord && afterWord[1] === second) {
                return pattern;
            }
        }

        // Check if current word is second part and previous word is first part
        if (word === second) {
            const beforeWord = line.substring(0, wordStart).match(/([a-z_]+)\s+$/);
            if (beforeWord && beforeWord[1] === first) {
                return pattern;
            }
        }
    }

    // Return single word
    return word;
}

// Context-specific documentation for 'else'
const ELSE_FOR_REQUIRE = {
    title: 'else (require)',
    description: 'Specifies the error value when a require condition fails.',
    syntax: '`require condition else error_expr`',
    example: `require user.is_authenticated() else "unauthorized"`
};

const ELSE_FOR_TRY_WHEN = {
    title: 'else (try when)',
    description: 'Fallback branch when all try when conditions are false. Must provide a value or error.',
    syntax: '`try when cond { expr } else { fallback }`',
    example: `handle! {
    try when n > 0 { positive_case()? }
    else { default_case() }
}`
};

// Detect if 'else' is used with 'require' or 'try when'
function getElseContext(document, position) {
    const text = document.getText();
    const offset = document.offsetAt(position);

    // Look backwards to find context
    const beforeText = text.substring(Math.max(0, offset - 500), offset);

    // Check if we're on the same line as 'require'
    const lines = beforeText.split('\n');
    const currentLineStart = beforeText.lastIndexOf('\n') + 1;
    const currentLine = beforeText.substring(currentLineStart);

    if (/\brequire\b/.test(currentLine)) {
        return 'require';
    }

    // Check for try when pattern (may be on previous lines)
    // Look for 'try when' or 'else when' followed by braces, then our else
    const tryWhenPattern = /\b(try\s+when|else\s+when)\b[^}]*\}\s*$/;
    if (tryWhenPattern.test(beforeText.trimEnd())) {
        return 'try_when';
    }

    // Also check if we're continuing an else when chain
    if (/\belse\s+when\b/.test(beforeText.substring(beforeText.length - 200))) {
        return 'try_when';
    }

    // Default - show generic
    return null;
}

// Create hover content for a keyword
function createHoverContent(keyword, document, position) {
    // Special handling for 'else' based on context
    if (keyword === 'else') {
        const context = getElseContext(document, position);
        let doc;
        if (context === 'require') {
            doc = ELSE_FOR_REQUIRE;
        } else if (context === 'try_when') {
            doc = ELSE_FOR_TRY_WHEN;
        } else {
            doc = KEYWORD_DOCS[keyword];
        }
        if (!doc) return null;

        const md = new vscode.MarkdownString();
        md.isTrusted = true;
        md.supportHtml = true;
        md.appendMarkdown(`## ${doc.title}\n\n`);
        md.appendMarkdown(`${doc.description}\n\n`);
        md.appendMarkdown(`**Syntax:** ${doc.syntax}\n\n`);
        md.appendMarkdown(`**Example:**\n\`\`\`rust\n${doc.example}\n\`\`\``);
        return new vscode.Hover(md);
    }

    const doc = KEYWORD_DOCS[keyword];
    if (!doc) return null;

    const md = new vscode.MarkdownString();
    md.isTrusted = true;
    md.supportHtml = true;

    md.appendMarkdown(`## ${doc.title}\n\n`);
    md.appendMarkdown(`${doc.description}\n\n`);
    md.appendMarkdown(`**Syntax:** ${doc.syntax}\n\n`);
    md.appendMarkdown(`**Example:**\n\`\`\`rust\n${doc.example}\n\`\`\``);

    return new vscode.Hover(md);
}

// Find the handle-this-lsp binary
function findLspBinary() {
    const config = vscode.workspace.getConfiguration('handleThis');
    const customPath = config.get('lsp.serverPath');

    console.log('handle-this-lsp: Looking for binary, customPath =', customPath);

    if (customPath && fs.existsSync(customPath)) {
        console.log('handle-this-lsp: Found at custom path');
        return customPath;
    }

    const homeDir = process.env.HOME || process.env.USERPROFILE;
    const candidates = [
        path.join(homeDir, '.cargo', 'bin', 'handle-this-lsp'),
        path.join(homeDir, '.local', 'bin', 'handle-this-lsp'),
        path.join(homeDir, '.cargo', 'bin', 'handle-this-lsp.exe'),
    ];

    for (const candidate of candidates) {
        if (fs.existsSync(candidate)) return candidate;
    }

    try {
        const which = process.platform === 'win32' ? 'where' : 'which';
        const result = execSync(`${which} handle-this-lsp`, { encoding: 'utf8' }).trim();
        if (result && fs.existsSync(result.split('\n')[0])) return result.split('\n')[0];
    } catch (e) {}

    return null;
}

// Send LSP message
function sendLspMessage(msg) {
    if (!lspProcess || !lspProcess.stdin.writable) return;
    const json = JSON.stringify(msg);
    const frame = `Content-Length: ${Buffer.byteLength(json)}\r\n\r\n${json}`;
    lspProcess.stdin.write(frame);
}

// Send LSP request and wait for response
function sendLspRequest(method, params) {
    return new Promise((resolve, reject) => {
        const id = requestId++;
        pendingRequests.set(id, { resolve, reject, timeout: setTimeout(() => {
            pendingRequests.delete(id);
            reject(new Error('LSP request timeout'));
        }, 30000)});
        sendLspMessage({ jsonrpc: '2.0', id, method, params });
    });
}

// Send LSP notification (no response expected)
function sendLspNotification(method, params) {
    sendLspMessage({ jsonrpc: '2.0', method, params });
}

// Handle incoming LSP messages
function handleLspMessage(msg) {
    if (msg.id !== undefined && pendingRequests.has(msg.id)) {
        const pending = pendingRequests.get(msg.id);
        clearTimeout(pending.timeout);
        pendingRequests.delete(msg.id);
        if (msg.error) pending.reject(new Error(msg.error.message));
        else pending.resolve(msg.result);
    }
    // Handle server notifications (diagnostics, etc.)
    if (msg.method === 'textDocument/publishDiagnostics' && msg.params) {
        const uri = vscode.Uri.parse(msg.params.uri);
        const diagnostics = (msg.params.diagnostics || []).map(d => {
            const range = new vscode.Range(
                d.range.start.line, d.range.start.character,
                d.range.end.line, d.range.end.character
            );
            const diag = new vscode.Diagnostic(range, d.message,
                d.severity === 1 ? vscode.DiagnosticSeverity.Error :
                d.severity === 2 ? vscode.DiagnosticSeverity.Warning :
                d.severity === 3 ? vscode.DiagnosticSeverity.Information :
                vscode.DiagnosticSeverity.Hint
            );
            if (d.source) diag.source = d.source;
            if (d.code) diag.code = d.code;
            return diag;
        });
        if (lspDiagnostics) lspDiagnostics.set(uri, diagnostics);
    }
}

// Start the LSP process
async function startLspClient(context) {
    const config = vscode.workspace.getConfiguration('handleThis');
    console.log('handle-this-lsp: enabled =', config.get('lsp.enabled'));
    if (!config.get('lsp.enabled')) return;

    const serverPath = findLspBinary();
    if (!serverPath) {
        vscode.window.showWarningMessage(
            'handle-this-lsp binary not found. Set handleThis.lsp.serverPath in settings.'
        );
        return;
    }

    lspDiagnostics = vscode.languages.createDiagnosticCollection('handle-this');
    context.subscriptions.push(lspDiagnostics);

    lspProcess = spawn(serverPath, [], { stdio: ['pipe', 'pipe', 'pipe'] });

    let buffer = '';
    lspProcess.stdout.on('data', (data) => {
        buffer += data.toString();
        while (true) {
            const headerEnd = buffer.indexOf('\r\n\r\n');
            if (headerEnd === -1) break;
            const header = buffer.substring(0, headerEnd);
            const match = header.match(/Content-Length:\s*(\d+)/i);
            if (!match) { buffer = buffer.substring(headerEnd + 4); continue; }
            const contentLength = parseInt(match[1], 10);
            const contentStart = headerEnd + 4;
            if (buffer.length < contentStart + contentLength) break;
            const content = buffer.substring(contentStart, contentStart + contentLength);
            buffer = buffer.substring(contentStart + contentLength);
            try { handleLspMessage(JSON.parse(content)); } catch (e) {}
        }
    });

    lspProcess.stderr.on('data', (data) => {
        console.log('handle-this-lsp:', data.toString());
    });

    lspProcess.on('exit', (code) => {
        console.log('handle-this-lsp exited with code', code);
        lspProcess = null;
    });

    // Initialize
    const workspaceFolders = vscode.workspace.workspaceFolders;
    const rootUri = workspaceFolders && workspaceFolders.length > 0
        ? workspaceFolders[0].uri.toString() : null;

    try {
        await sendLspRequest('initialize', {
            processId: process.pid,
            rootUri: rootUri,
            capabilities: {
                textDocument: {
                    hover: { contentFormat: ['markdown', 'plaintext'] },
                    completion: { completionItem: { snippetSupport: true } },
                    publishDiagnostics: { relatedInformation: true }
                }
            }
        });
        sendLspNotification('initialized', {});

        // Open all currently open Rust documents
        for (const doc of vscode.workspace.textDocuments) {
            if (doc.languageId === 'rust') {
                sendLspNotification('textDocument/didOpen', {
                    textDocument: {
                        uri: doc.uri.toString(),
                        languageId: 'rust',
                        version: doc.version,
                        text: doc.getText()
                    }
                });
            }
        }

        // Register VS Code providers that query the LSP
        registerLspProviders(context);

        vscode.window.showInformationMessage('handle-this LSP started');
    } catch (e) {
        vscode.window.showErrorMessage(`Failed to start handle-this LSP: ${e.message}`);
        stopLspClient();
    }
}

// Stop the LSP process
function stopLspClient() {
    if (lspProcess) {
        try {
            sendLspNotification('shutdown', null);
            sendLspNotification('exit', null);
        } catch (e) {}
        lspProcess.kill();
        lspProcess = null;
    }
    pendingRequests.clear();
}

// Register LSP-backed providers
function registerLspProviders(context) {
    // Hover provider
    const hoverProvider = vscode.languages.registerHoverProvider('rust', {
        async provideHover(document, position, token) {
            if (!lspProcess) return null;
            try {
                const result = await sendLspRequest('textDocument/hover', {
                    textDocument: { uri: document.uri.toString() },
                    position: { line: position.line, character: position.character }
                });
                if (!result || !result.contents) return null;

                let contents;
                if (typeof result.contents === 'string') {
                    contents = new vscode.MarkdownString(result.contents);
                } else if (result.contents.kind === 'markdown') {
                    contents = new vscode.MarkdownString(result.contents.value);
                } else if (result.contents.value) {
                    contents = new vscode.MarkdownString(result.contents.value);
                } else if (Array.isArray(result.contents)) {
                    contents = new vscode.MarkdownString(result.contents.map(c =>
                        typeof c === 'string' ? c : c.value
                    ).join('\n\n'));
                } else {
                    contents = new vscode.MarkdownString(String(result.contents));
                }

                let range;
                if (result.range) {
                    range = new vscode.Range(
                        result.range.start.line, result.range.start.character,
                        result.range.end.line, result.range.end.character
                    );
                }
                return new vscode.Hover(contents, range);
            } catch (e) {
                console.log('handle-this-lsp hover error:', e);
                return null;
            }
        }
    });

    // Completion provider
    const completionProvider = vscode.languages.registerCompletionItemProvider('rust', {
        async provideCompletionItems(document, position, token, context) {
            if (!lspProcess) return null;
            try {
                const result = await sendLspRequest('textDocument/completion', {
                    textDocument: { uri: document.uri.toString() },
                    position: { line: position.line, character: position.character }
                });
                if (!result) return null;

                const items = Array.isArray(result) ? result : (result.items || []);
                return items.map(item => {
                    const ci = new vscode.CompletionItem(item.label, item.kind || vscode.CompletionItemKind.Text);
                    if (item.detail) ci.detail = item.detail;
                    if (item.documentation) ci.documentation = typeof item.documentation === 'string'
                        ? item.documentation
                        : new vscode.MarkdownString(item.documentation.value);
                    if (item.insertText) ci.insertText = item.insertText;
                    if (item.filterText) ci.filterText = item.filterText;
                    if (item.sortText) ci.sortText = item.sortText;
                    return ci;
                });
            } catch (e) {
                console.log('handle-this-lsp completion error:', e);
                return null;
            }
        }
    }, '.', ':');

    // Definition provider
    const definitionProvider = vscode.languages.registerDefinitionProvider('rust', {
        async provideDefinition(document, position, token) {
            if (!lspProcess) return null;
            try {
                const result = await sendLspRequest('textDocument/definition', {
                    textDocument: { uri: document.uri.toString() },
                    position: { line: position.line, character: position.character }
                });
                if (!result) return null;

                const locations = Array.isArray(result) ? result : [result];
                return locations.map(loc => {
                    const uri = vscode.Uri.parse(loc.uri || loc.targetUri);
                    const range = loc.range || loc.targetRange;
                    return new vscode.Location(uri, new vscode.Range(
                        range.start.line, range.start.character,
                        range.end.line, range.end.character
                    ));
                });
            } catch (e) {
                console.log('handle-this-lsp definition error:', e);
                return null;
            }
        }
    });

    // References provider
    const referencesProvider = vscode.languages.registerReferenceProvider('rust', {
        async provideReferences(document, position, context, token) {
            if (!lspProcess) return null;
            try {
                const result = await sendLspRequest('textDocument/references', {
                    textDocument: { uri: document.uri.toString() },
                    position: { line: position.line, character: position.character },
                    context: { includeDeclaration: true }
                });
                if (!result) return null;

                return result.map(loc => {
                    const uri = vscode.Uri.parse(loc.uri);
                    return new vscode.Location(uri, new vscode.Range(
                        loc.range.start.line, loc.range.start.character,
                        loc.range.end.line, loc.range.end.character
                    ));
                });
            } catch (e) {
                console.log('handle-this-lsp references error:', e);
                return null;
            }
        }
    });

    context.subscriptions.push(hoverProvider, completionProvider, definitionProvider, referencesProvider);
}

function activate(context) {
    ctx = context;
    load(); apply();
    context.subscriptions.push(vscode.commands.registerCommand('handleThis.openColorPicker', openPicker));

    // Register hover provider for Rust files
    const hoverProvider = vscode.languages.registerHoverProvider('rust', {
        provideHover(document, position, token) {
            const keyword = getKeywordAtPosition(document, position);
            // Allow 'else' even if not in KEYWORD_DOCS directly (context-specific)
            if (!keyword || (!KEYWORD_DOCS[keyword] && keyword !== 'else')) return null;

            // Only show hover if inside a handle! macro
            if (!isInsideHandleMacro(document, position)) return null;

            return createHoverContent(keyword, document, position);
        }
    });

    context.subscriptions.push(hoverProvider);

    // Start LSP if enabled
    startLspClient(context);

    // Document sync for LSP
    context.subscriptions.push(
        vscode.workspace.onDidOpenTextDocument((doc) => {
            if (lspProcess && doc.languageId === 'rust') {
                sendLspNotification('textDocument/didOpen', {
                    textDocument: {
                        uri: doc.uri.toString(),
                        languageId: 'rust',
                        version: doc.version,
                        text: doc.getText()
                    }
                });
            }
        }),
        vscode.workspace.onDidChangeTextDocument((e) => {
            if (lspProcess && e.document.languageId === 'rust') {
                sendLspNotification('textDocument/didChange', {
                    textDocument: {
                        uri: e.document.uri.toString(),
                        version: e.document.version
                    },
                    contentChanges: [{ text: e.document.getText() }]
                });
            }
        }),
        vscode.workspace.onDidCloseTextDocument((doc) => {
            if (lspProcess && doc.languageId === 'rust') {
                sendLspNotification('textDocument/didClose', {
                    textDocument: { uri: doc.uri.toString() }
                });
            }
        })
    );

    // Watch for configuration changes
    context.subscriptions.push(
        vscode.workspace.onDidChangeConfiguration(async (e) => {
            if (e.affectsConfiguration('handleThis')) {
                stopLspClient();
                await startLspClient(context);
            }
        })
    );
}

function deactivate() {
    stopLspClient();
}
module.exports = { activate, deactivate };
