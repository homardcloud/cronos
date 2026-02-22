// Cronos UI — Tauri IPC frontend

const { invoke } = window.__TAURI__.core;

// ---- State ----
let trackingPaused = false;
let collectorsRunning = true;
let authenticated = false;
let sending = false;

// ---- DOM refs ----
const chatMessages = document.getElementById('chat-messages');
const chatInput = document.getElementById('chat-input');
const btnSend = document.getElementById('btn-send');
const btnPause = document.getElementById('btn-pause');
const btnKill = document.getElementById('btn-kill');
const btnAuth = document.getElementById('btn-auth');
const trackingStatus = document.getElementById('tracking-status');
const trackingIndicator = document.getElementById('tracking-indicator');

// ---- Markdown-lite renderer ----
function renderMarkdown(text) {
  // Escape HTML
  let html = text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');

  // Code blocks (``` ... ```)
  html = html.replace(/```(\w*)\n([\s\S]*?)```/g, (_m, _lang, code) => {
    return `<pre><code>${code.trim()}</code></pre>`;
  });

  // Inline code
  html = html.replace(/`([^`]+)`/g, '<code>$1</code>');

  // Bold
  html = html.replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>');

  // Italic
  html = html.replace(/(?<!\*)\*([^*]+)\*(?!\*)/g, '<em>$1</em>');

  // Unordered lists
  html = html.replace(/^[\-\*] (.+)$/gm, '<li>$1</li>');
  html = html.replace(/((?:<li>.*<\/li>\n?)+)/g, '<ul>$1</ul>');

  // Ordered lists
  html = html.replace(/^\d+\. (.+)$/gm, '<li>$1</li>');

  // Paragraphs (double newlines)
  html = html.replace(/\n\n+/g, '</p><p>');
  html = `<p>${html}</p>`;

  // Clean up empty paragraphs
  html = html.replace(/<p>\s*<\/p>/g, '');
  html = html.replace(/<p>\s*(<pre>)/g, '$1');
  html = html.replace(/(<\/pre>)\s*<\/p>/g, '$1');
  html = html.replace(/<p>\s*(<ul>)/g, '$1');
  html = html.replace(/(<\/ul>)\s*<\/p>/g, '$1');

  return html;
}

// ---- Message display ----
function clearWelcome() {
  const w = chatMessages.querySelector('.welcome-msg');
  if (w) w.remove();
}

function addMessage(role, content) {
  clearWelcome();
  const div = document.createElement('div');
  div.className = `message ${role}`;

  if (role === 'assistant') {
    div.innerHTML = renderMarkdown(content);
  } else if (role === 'error') {
    div.textContent = content;
  } else {
    div.textContent = content;
  }

  chatMessages.appendChild(div);
  chatMessages.scrollTop = chatMessages.scrollHeight;
  return div;
}

function addThinking() {
  clearWelcome();
  const div = document.createElement('div');
  div.className = 'thinking';
  div.innerHTML = '<span></span><span></span><span></span>';
  chatMessages.appendChild(div);
  chatMessages.scrollTop = chatMessages.scrollHeight;
  return div;
}

function removeThinking(el) {
  if (el && el.parentNode) el.parentNode.removeChild(el);
}

// ---- Tracking UI ----
function updateTrackingUI() {
  if (!collectorsRunning) {
    trackingStatus.textContent = 'Stopped';
    trackingStatus.className = 'status-badge stopped';
    trackingIndicator.className = 'indicator stopped';
    btnPause.disabled = true;
    btnKill.innerHTML = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none">
      <polygon points="4,2 12,7 4,12" fill="currentColor"/>
    </svg>`;
    btnKill.title = 'Start collectors';
    btnKill.classList.remove('btn-danger');
    btnKill.classList.add('active');
  } else if (trackingPaused) {
    trackingStatus.textContent = 'Paused';
    trackingStatus.className = 'status-badge paused';
    trackingIndicator.className = 'indicator paused';
    btnPause.disabled = false;
    btnPause.classList.add('active');
    btnPause.innerHTML = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none">
      <polygon points="4,2 12,7 4,12" fill="currentColor"/>
    </svg>`;
    btnPause.title = 'Resume tracking';
    btnKill.innerHTML = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none">
      <rect x="2" y="6" width="10" height="2" rx="0.5" fill="currentColor"/>
    </svg>`;
    btnKill.title = 'Kill collectors';
    btnKill.classList.add('btn-danger');
    btnKill.classList.remove('active');
  } else {
    trackingStatus.textContent = 'Active';
    trackingStatus.className = 'status-badge active';
    trackingIndicator.className = 'indicator active';
    btnPause.disabled = false;
    btnPause.classList.remove('active');
    btnPause.innerHTML = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none">
      <rect x="3" y="2" width="3" height="10" rx="0.5" fill="currentColor"/>
      <rect x="8" y="2" width="3" height="10" rx="0.5" fill="currentColor"/>
    </svg>`;
    btnPause.title = 'Pause tracking';
    btnKill.innerHTML = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none">
      <rect x="2" y="6" width="10" height="2" rx="0.5" fill="currentColor"/>
    </svg>`;
    btnKill.title = 'Kill collectors';
    btnKill.classList.add('btn-danger');
    btnKill.classList.remove('active');
  }
}

// ---- Event handlers ----
async function sendMessage() {
  const text = chatInput.value.trim();
  if (!text || sending) return;

  sending = true;
  chatInput.value = '';
  chatInput.style.height = 'auto';
  btnSend.disabled = true;

  addMessage('user', text);
  const thinking = addThinking();

  try {
    const response = await invoke('send_message', { text });
    removeThinking(thinking);
    addMessage('assistant', response);
  } catch (e) {
    removeThinking(thinking);
    addMessage('error', String(e));
  }

  sending = false;
  btnSend.disabled = !chatInput.value.trim();
  chatInput.focus();
}

async function togglePause() {
  try {
    const result = await invoke('set_tracking_paused', { paused: !trackingPaused });
    trackingPaused = result;
    updateTrackingUI();
  } catch (e) {
    addMessage('error', `Pause toggle failed: ${e}`);
  }
}

async function toggleCollectors() {
  try {
    if (collectorsRunning) {
      await invoke('kill_collectors');
      collectorsRunning = false;
      trackingPaused = false;
    } else {
      await invoke('start_collectors');
      collectorsRunning = true;
      trackingPaused = false;
    }
    updateTrackingUI();
  } catch (e) {
    addMessage('error', `Collector toggle failed: ${e}`);
  }
}

async function toggleAuth() {
  try {
    if (authenticated) {
      await invoke('logout');
      authenticated = false;
      btnAuth.classList.remove('authenticated');
      btnAuth.title = 'Login';
    } else {
      const result = await invoke('login');
      authenticated = true;
      btnAuth.classList.add('authenticated');
      btnAuth.title = `Logged in: ${result}`;
    }
  } catch (e) {
    addMessage('error', `Auth failed: ${e}`);
  }
}

async function checkStatus() {
  try {
    const status = await invoke('get_status');
    if (status.authenticated) {
      authenticated = true;
      btnAuth.classList.add('authenticated');
      btnAuth.title = 'Logged in';
    }
    if (status.tracking_paused !== undefined) {
      trackingPaused = status.tracking_paused;
    }
    updateTrackingUI();
  } catch (_e) {
    // Daemon may not be running yet
  }
}

// ---- Input handling ----
chatInput.addEventListener('input', () => {
  btnSend.disabled = !chatInput.value.trim() || sending;
  // Auto-resize textarea
  chatInput.style.height = 'auto';
  chatInput.style.height = Math.min(chatInput.scrollHeight, 120) + 'px';
});

chatInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    sendMessage();
  }
});

btnSend.addEventListener('click', sendMessage);
btnPause.addEventListener('click', togglePause);
btnKill.addEventListener('click', toggleCollectors);
btnAuth.addEventListener('click', toggleAuth);

// ---- Init ----
checkStatus();
chatInput.focus();
