// RuSense — frontend do cockpit. Toda mutação passa pelos comandos Tauri;
// o estado renderizado vem sempre do backend (UiState), nunca de palpite local.
(() => {
  'use strict';

  const $ = s => document.querySelector(s);
  const invoke = window.__TAURI__
    ? window.__TAURI__.core.invoke
    : () => Promise.reject('Tauri indisponível (abra pelo rusense-gui)');

  const MAX_RPM = 6000, ARC = 75; // arco de 270° (pathLength 100)
  const POLL_MS = 2000;

  let current = null;       // último UiState renderizado
  let dragging = false;     // true enquanto um slider de fan está sendo arrastado
  let lastPollError = null; // evita spam de toast quando o poll falha em série
  let gen = 0;              // carimbo do invoke mais recente emitido
  let renderedGen = 0;      // carimbo do último UiState renderizado

  // ---------- toast (superfície de status/erro) ----------
  let toastTimer;
  function toast(text) {
    $('#toast-msg').textContent = text;
    const t = $('#toast');
    t.classList.add('show');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => t.classList.remove('show'), 3200);
  }

  // ---------- render ----------
  function setGauge(which, rpm) {
    rpm = Math.max(0, Math.min(MAX_RPM, Math.round(rpm)));
    $('#arc-' + which).style.strokeDasharray = (ARC * rpm / MAX_RPM) + ' 100';
    $('#rpm-' + which).textContent = rpm;
  }

  function renderProfiles(p) {
    const wrap = $('#profiles');
    const names = new Set(p.available);
    const covered = new Set();
    for (const b of [...wrap.querySelectorAll('.profile')]) {
      const known = names.has(b.dataset.v);
      if (!known && b.dataset.extra) { b.remove(); continue; }
      b.hidden = !known;
      if (known) covered.add(b.dataset.v);
      const active = b.dataset.v === p.active;
      b.classList.toggle('active', active);
      b.setAttribute('aria-pressed', String(active));
    }
    // Perfis que o driver reporta além dos quatro conhecidos: botão simples.
    for (const name of p.available) {
      if (covered.has(name)) continue;
      const b = document.createElement('button');
      b.className = 'profile';
      b.dataset.v = name;
      b.dataset.extra = '1';
      b.append(name);
      // <small> vazio mantém o grid interno alinhado com os botões estáticos.
      b.appendChild(document.createElement('small'));
      const active = name === p.active;
      b.classList.toggle('active', active);
      b.setAttribute('aria-pressed', String(active));
      wrap.appendChild(b);
    }
  }

  function renderFan(fan) {
    for (const b of document.querySelectorAll('.mode')) {
      const on = b.dataset.v === fan.mode;
      b.classList.toggle('active', on);
      b.setAttribute('aria-pressed', String(on));
    }
    const custom = fan.mode === 'custom';
    $('#s-cpu').disabled = $('#s-gpu').disabled = !custom;
    // Os sliders são estado LOCAL (default 50) e só sincronizam do backend em
    // modo custom. Em auto/max o DTO reporta os duties aliased (0,0 / 100,100);
    // copiá-los pros sliders faria o próximo clique em "Custom" enviar 0,0 —
    // que FanMode::custom normaliza de volta pra Auto, deixando o modo custom
    // inalcançável (mesmo padrão do TUI: tui/src/app.rs).
    if (custom && !dragging) {
      $('#s-cpu').value = fan.cpu; $('#o-cpu').textContent = fan.cpu + '%';
      $('#s-gpu').value = fan.gpu; $('#o-gpu').textContent = fan.gpu + '%';
    }
  }

  function renderPower(p) {
    for (const [id, on] of [['#t-limiter', p.limiter], ['#t-backlight', p.backlight]]) {
      const t = $(id);
      t.classList.toggle('on', on);
      t.setAttribute('aria-pressed', String(on));
    }
    for (const b of document.querySelectorAll('.seg')) {
      const on = +b.dataset.v === p.usb;
      b.classList.toggle('active', on);
      b.setAttribute('aria-pressed', String(on));
    }
  }

  function render(s) {
    current = s;
    setGauge('cpu', s.telemetry.cpu_rpm);
    setGauge('gpu', s.telemetry.gpu_rpm);
    $('#temp-cpu').textContent = Math.round(s.telemetry.temps[0]) + '°';
    $('#temp-gpu').textContent = Math.round(s.telemetry.temps[1]) + '°';
    $('#temp-sys').textContent = Math.round(s.telemetry.temps[2]) + '°';

    renderProfiles(s.profiles);
    renderFan(s.fan);
    renderPower(s.power);

    $('#card-fans').hidden = !s.caps.fan_control;
    $('#card-energy').hidden = !s.caps.power;
    $('#st-rgb').textContent = 'RGB 4 zonas: ' +
      (s.caps.four_zone_kb ? 'disponível' : 'indisponível neste modelo');

    $('#st-profile').textContent = s.profiles.active;
    $('#st-fans').textContent =
      s.fan.mode === 'custom' ? s.fan.cpu + ',' + s.fan.gpu : s.fan.mode;
    $('#st-batt').textContent =
      s.telemetry.battery_pct + '% · ' + s.telemetry.battery_status.toLowerCase();
    $('#st-mock').hidden = !s.mock;
    // O chip estático da titlebar mentiria em --mock: segue o backend real.
    $('#st-driver').textContent = s.mock ? 'modo simulado' : 'linuwu_sense carregado';
  }

  // ---------- bridge ----------
  // Cada invoke recebe um carimbo de geração; uma resposta que chega depois
  // de outra mais nova ser renderizada é descartada (poll lento não pode
  // sobrescrever o UiState devolvido por uma ação, e vice-versa).
  function commit(g, s) {
    if (g < renderedGen) return;
    renderedGen = g;
    render(s);
  }

  function call(cmd, args, okMsg) {
    const g = ++gen;
    invoke(cmd, args)
      .then(s => { commit(g, s); if (okMsg) toast(okMsg); })
      .catch(e => { toast(String(e)); refresh(); });
  }

  function refresh() {
    const g = ++gen;
    invoke('state')
      .then(s => { lastPollError = null; commit(g, s); })
      .catch(e => {
        const msg = String(e);
        if (msg !== lastPollError) toast(msg);
        lastPollError = msg;
      });
  }

  // ---------- controles ----------
  $('#profiles').addEventListener('click', e => {
    const b = e.target.closest('.profile'); if (!b) return;
    call('set_profile', { name: b.dataset.v }, 'perfil → ' + b.dataset.v);
  });

  function applyFan(mode) {
    const cpu = +$('#s-cpu').value, gpu = +$('#s-gpu').value;
    const label = mode === 'custom' ? cpu + ',' + gpu : mode;
    call('set_fan', { mode, cpu, gpu }, 'fans → ' + label);
  }

  $('#modes').addEventListener('click', e => {
    const b = e.target.closest('.mode'); if (!b) return;
    if (b.dataset.v === 'custom') {
      // 0,0 e 100,100 são aliases do driver (auto/max): FanMode::custom
      // normalizaria de volta e Custom ficaria inalcançável na sessão.
      // Se AMBOS os sliders estão num extremo aliased, reentra em 50,50.
      const cpu = +$('#s-cpu').value, gpu = +$('#s-gpu').value;
      if ((cpu === 0 && gpu === 0) || (cpu === 100 && gpu === 100)) {
        $('#s-cpu').value = $('#s-gpu').value = 50;
        $('#o-cpu').textContent = $('#o-gpu').textContent = '50%';
      }
    }
    applyFan(b.dataset.v);
  });

  for (const k of ['cpu', 'gpu']) {
    const s = $('#s-' + k);
    s.addEventListener('pointerdown', () => { dragging = true; });
    s.addEventListener('input', e => {
      dragging = true;
      $('#o-' + k).textContent = e.target.value + '%';
    });
    s.addEventListener('change', () => { dragging = false; applyFan('custom'); });
    s.addEventListener('pointerup', () => { dragging = false; });
    s.addEventListener('pointercancel', () => { dragging = false; });
  }

  $('#t-limiter').addEventListener('click', () => {
    if (!current) return;
    const p = current.power;
    call('set_power', { limiter: !p.limiter, usb: p.usb, backlight: p.backlight },
      'limite 80% → ' + (!p.limiter ? 'on' : 'off'));
  });

  $('#t-backlight').addEventListener('click', () => {
    if (!current) return;
    const p = current.power;
    call('set_power', { limiter: p.limiter, usb: p.usb, backlight: !p.backlight },
      'timeout do backlight → ' + (!p.backlight ? 'on' : 'off'));
  });

  $('#usb-segs').addEventListener('click', e => {
    const b = e.target.closest('.seg'); if (!b || !current) return;
    const p = current.power;
    const usb = +b.dataset.v;
    call('set_power', { limiter: p.limiter, usb, backlight: p.backlight },
      'usb charging → ' + (usb === 0 ? 'off' : usb + '%'));
  });

  // ---------- ciclo de vida ----------
  refresh();
  setInterval(refresh, POLL_MS);
})();
