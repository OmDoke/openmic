/* eslint-disable no-undef */
/**
 * Onboarding wizard controller.
 *
 * Drives the 5-step flow rendered in onboarding.html and persists
 * everything via the electronAPI bridge exposed by preload.js:
 *
 *   1. Welcome
 *   2. Gemini API key entry + live connection test
 *   3. Cloud speech provider choice (Deepgram / Groq / Skip)
 *   4. Star-the-repo prompt + summary
 */

(function () {
  'use strict';

  // â”€â”€ DOM refs â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
  const $ = (sel) => document.querySelector(sel);
  const $$ = (sel) => document.querySelectorAll(sel);

  const screens = $$('.screen');
  const stepperDots = $$('.step-dot');
  const stepBadge = $('#stepBadge');
  const backBtn = $('#backBtn');
  const nextBtn = $('#nextBtn');
  const nav = $('#wizard .nav'); // the centered nav container

  // â”€â”€ State â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
  const state = {
    step: 0,
    geminiKey: '',
    geminiConfigured: false, // a key already exists in .env from a prior run
    speechProvider: null, // 'groq' | 'deepgram' | 'skip'
    groqSpeechKey: '',
    groqSpeechModel: 'whisper-large-v3',
    deepgramSpeechKey: '',
    deepgramSpeechModel: 'nova-3',
    finished: false,
  };

  // Screens are: welcome â†’ apikey â†’ speech â†’ finish
  const stepScreens = ['welcome', 'apikey', 'speech'];

  // â”€â”€ Step rendering â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
  function totalSteps() {
    return stepScreens.length + 1;
  }

  function refreshStepper() {
    const total = totalSteps();
    const current = state.step + 1;
    stepBadge.textContent = `Step ${current} of ${total}`;
    stepperDots.forEach((dot, i) => {
      dot.classList.remove('active', 'done');
      if (i < state.step) dot.classList.add('done');
      else if (i === state.step) dot.classList.add('active');
    });
  }

  function showScreen(name) {
    screens.forEach((s) => {
      s.classList.toggle('active', s.dataset.screen === name);
    });
    // Welcome screen uses an inline hero CTA â€” hide the regular nav row.
    const wizardEl = document.getElementById('wizard');
    if (wizardEl) {
      wizardEl.classList.toggle('welcome-active', name === 'welcome');
    }
    refreshStepper();
    backBtn.style.visibility = state.step === 0 ? 'hidden' : 'visible';
    nextBtn.disabled = false;
    nextBtn.classList.remove('success');
    nextBtn.classList.add('primary');
    // The primary action label changes by step
    if (name === 'welcome') nextBtn.innerHTML = 'Get started <i class="fas fa-arrow-right"></i>';
    else if (name === 'finish') nextBtn.innerHTML = 'Finish <i class="fas fa-check"></i>';
    else nextBtn.innerHTML = 'Continue <i class="fas fa-arrow-right"></i>';
  }

  function navigate(direction) {
    const order = computeScreenOrder();
    const idx = order.indexOf(currentScreenName());
    const next = direction === 'next' ? idx + 1 : idx - 1;
    if (next < 0 || next >= order.length) return;
    state.step = orderScreenToStep(order[next]);
    showScreen(order[next]);
  }

  function currentScreenName() {
    const active = Array.from(screens).find((s) => s.classList.contains('active'));
    return active ? active.dataset.screen : 'welcome';
  }

  function computeScreenOrder() {
    return ['welcome', 'apikey', 'speech', 'finish'];
  }

  // Map a screen name to its position in the stepper (0..n).
  function orderScreenToStep(name) {
    return computeScreenOrder().indexOf(name);
  }

  // â”€â”€ Validation gates before "Continue" â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
  function canAdvance() {
    const name = currentScreenName();
    switch (name) {
      case 'welcome':
        return true;
      case 'apikey':
        // A key already in .env is enough â€” don't force a re-entry.
        return !!state.geminiKey.trim() || state.geminiConfigured;
      case 'speech':
        if (state.speechProvider === 'groq') {
          return !!state.groqSpeechKey.trim();
        }
        if (state.speechProvider === 'deepgram') {
          return !!state.deepgramSpeechKey.trim();
        }
        return !!state.speechProvider;
      case 'finish':
        return true;
      default:
        return true;
    }
  }

  // â”€â”€ Wire up: API key â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
  const geminiInput = $('#geminiKey');
  const toggleVis = $('#toggleVis');
  const keyStatus = $('#keyStatus');

  function setKeyStatus(state_, text) {
    keyStatus.className = `status-pill ${state_}`;
    keyStatus.style.display = 'inline-flex';
    const icon = keyStatus.querySelector('i');
    const txt = keyStatus.querySelector('.text');
    if (state_ === 'testing') {
      icon.className = 'fas fa-circle-notch fa-spin';
    } else if (state_ === 'success') {
      icon.className = 'fas fa-check-circle';
    } else if (state_ === 'error') {
      icon.className = 'fas fa-circle-xmark';
    } else {
      icon.className = 'fas fa-circle-info';
    }
    txt.textContent = text;
  }

  geminiInput.addEventListener('input', () => {
    state.geminiKey = geminiInput.value.trim();
    if (!state.geminiKey) {
      keyStatus.style.display = 'none';
    } else if (keyStatus.classList.contains('success')) {
      // Keep success state â€” they had a valid key, may be editing
    } else {
      setKeyStatus('idle', 'Key entered');
    }
  });

  toggleVis.addEventListener('click', () => {
    const showing = geminiInput.type === 'text';
    geminiInput.type = showing ? 'password' : 'text';
    toggleVis.innerHTML = showing
      ? '<i class="fas fa-eye"></i>'
      : '<i class="fas fa-eye-slash"></i>';
  });

  // â”€â”€ Wire up: Speech choices â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
  const groqPanel = $('#groqPanel');
  const deepgramPanel = $('#deepgramPanel');
  const groqSpeechLabel = $('#groqSpeechLabel');
  const groqSpeechHint = $('#groqSpeechHint');
  const speechStatus = $('#speechStatus');

  $$('#speechChoices .choice-card').forEach((card) => {
    card.addEventListener('click', () => {
      const value = card.dataset.value;
      state.speechProvider = value;
      speechStatus.style.display = 'none';
      $$('#speechChoices .choice-card').forEach((c) => c.classList.remove('selected'));
      card.classList.add('selected');
      groqPanel.style.display = value === 'groq' || value === 'deepgram' ? 'block' : 'none';
      deepgramPanel.style.display = value === 'deepgram' ? 'block' : 'none';
      if (value === 'deepgram') {
        groqSpeechLabel.textContent = 'Groq Fallback API Key';
        groqSpeechHint.textContent = 'Optional. Used only if Deepgram transcription fails.';
      } else {
        groqSpeechLabel.textContent = 'Groq API Key';
        groqSpeechHint.innerHTML = 'Get a key at <a href="https://console.groq.com/keys" target="_blank" rel="noreferrer">console.groq.com/keys</a>.';
      }
    });
  });

  $('#groqSpeechKey').addEventListener('input', (e) => { state.groqSpeechKey = e.target.value.trim(); speechStatus.style.display = 'none'; });
  $('#groqSpeechModel').addEventListener('input', (e) => { state.groqSpeechModel = e.target.value.trim(); });
  $('#deepgramSpeechKey').addEventListener('input', (e) => { state.deepgramSpeechKey = e.target.value.trim(); speechStatus.style.display = 'none'; });
  $('#deepgramSpeechModel').addEventListener('input', (e) => { state.deepgramSpeechModel = e.target.value.trim(); });

  // â”€â”€ Wire up: Finish screen â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
  function populateSummary() {
    const rows = [];
    rows.push({
      label: '<i class="fas fa-key"></i> Gemini API',
      value: (state.geminiKey || state.geminiConfigured) ? 'Configured' : 'Missing',
      cls: (state.geminiKey || state.geminiConfigured) ? 'ok' : 'skip',
    });
    if (state.speechProvider === 'groq') {
      rows.push({
        label: '<i class="fas fa-microphone"></i> Speech',
        value: 'Groq cloud transcription',
        cls: 'ok',
      });
    } else if (state.speechProvider === 'deepgram') {
      rows.push({
        label: '<i class="fas fa-microphone"></i> Speech',
        value: state.groqSpeechKey ? 'Deepgram (Groq fallback configured)' : 'Deepgram',
        cls: 'ok',
      });
    } else {
      rows.push({
        label: '<i class="fas fa-microphone"></i> Speech',
        value: 'Skipped (configure later)',
        cls: 'skip',
      });
    }
    rows.push({
      label: '<i class="fas fa-file-lines"></i> Config saved to',
      value: '.env',
      cls: 'ok',
    });
    $('#summaryList').innerHTML = rows
      .map((r) => `
        <div class="summary-row">
          <div class="label">${r.label}</div>
          <div class="value ${r.cls}">${r.value}</div>
        </div>
      `)
      .join('');
  }

  $('#starBtn').addEventListener('click', () => {
    if (window.electronAPI && window.electronAPI.openExternal) {
      window.electronAPI.openExternal('https://github.com/TechyCSR/OpenCluely');
    } else {
      window.open('https://github.com/TechyCSR/OpenCluely', '_blank');
    }
  });
  $('#skipStarBtn').addEventListener('click', () => {
    // No-op â€” just visual closure
  });

  // â”€â”€ Wire up: Hero CTA (welcome screen) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
  // The big inline "Get Started" button on the welcome screen reuses
  // the existing nav-button handler so all validation, persistence,
  // and navigation logic stays in one place.
  const heroCtaBtn = $('#heroCtaBtn');
  if (heroCtaBtn) {
    heroCtaBtn.addEventListener('click', () => nextBtn.click());
  }

  // â”€â”€ Wire up: nav buttons â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
  nextBtn.addEventListener('click', async () => {
    const name = currentScreenName();
    if (!canAdvance()) {
      // Lightly nudge the user
      if (name === 'apikey') setKeyStatus('error', 'Enter a Gemini API key');
      if (name === 'speech') {
        const message = state.speechProvider === 'groq'
          ? 'Enter a Groq API key.'
          : state.speechProvider === 'deepgram'
            ? 'Enter a Deepgram API key. Groq fallback is optional.'
            : 'Choose a speech option to continue.';
        speechStatus.querySelector('.text').textContent = message;
        speechStatus.style.display = 'inline-flex';
      }
      return;
    }

    // Persist the speech choice; the Gemini key is saved above if needed.
    if (name === 'apikey' && state.geminiKey && window.electronAPI) {
      try {
        await window.electronAPI.saveSettings({ geminiKey: state.geminiKey });
      } catch (_) { /* surfaced elsewhere */ }
    }
    if (name === 'speech' && window.electronAPI) {
      try {
        const payload = {
          speechProvider:
            state.speechProvider === 'skip' ? 'disabled' : state.speechProvider,
        };
        if (state.speechProvider === 'groq') {
          payload.groqSpeechKey = state.groqSpeechKey;
          payload.groqSpeechModel = state.groqSpeechModel || 'whisper-large-v3';
        }
        if (state.speechProvider === 'deepgram') {
          payload.deepgramSpeechKey = state.deepgramSpeechKey;
          payload.deepgramSpeechModel = state.deepgramSpeechModel || 'nova-3';
          payload.groqSpeechKey = state.groqSpeechKey;
          payload.groqSpeechModel = state.groqSpeechModel || 'whisper-large-v3';
        }
        await window.electronAPI.saveSettings(payload);
      } catch (_) { /* surfaced elsewhere */ }
    }

    // Finish: close onboarding
    if (name === 'finish') {
      try {
        await window.electronAPI.completeFirstRun();
      } catch (_) { /* ignore */ }
      try {
        await window.electronAPI.closeOnboarding();
      } catch (_) { /* ignore */ }
      state.finished = true;
      return;
    }

    const order = computeScreenOrder();
    const idx = order.indexOf(name);
    const nextName = order[idx + 1];
    if (!nextName) return;

    // Compute new step index
    state.step = orderScreenToStep(nextName);
    showScreen(nextName);
    if (nextName === 'finish') populateSummary();

    // Re-render stepper with new total
    refreshStepper();
  });

  backBtn.addEventListener('click', () => {
    const name = currentScreenName();
    const order = computeScreenOrder();
    const idx = order.indexOf(name);
    const prevName = order[idx - 1];
    if (!prevName) return;
    state.step = orderScreenToStep(prevName);
    showScreen(prevName);
  });

  // â”€â”€ Boot â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
  showScreen('welcome');

  // Pre-populate Gemini key from existing .env (if any) so users with
  // a partial config don't have to retype.
  if (window.electronAPI && window.electronAPI.getFirstRunStatus) {
    window.electronAPI.getFirstRunStatus().then((s) => {
      if (s && s.geminiConfigured) {
        // We can't read the key back (settings returns empty for keys),
        // but we can mark status as success if the env file already has one
        // and let the user advance without retyping it.
        state.geminiConfigured = true;
        setKeyStatus('success', 'Already configured â€” click Continue');
        geminiInput.placeholder = 'â€¢â€¢â€¢â€¢â€¢â€¢â€¢â€¢â€¢â€¢â€¢â€¢â€¢â€¢â€¢â€¢ (already set)';
      }
    }).catch(() => {});
  }
})();
