/* Saturn Go shared appliance shell: sidebar/drawer navigation + theme toggle.
   Vanilla JS, no build step — loaded directly via <script src="/assets/js/saturn-shell.js"> */
(function (global) {
  'use strict';

  var NAV_SECTIONS = [
    {
      label: null,
      items: [
        { key: 'overview', label: 'Overview', href: './' },
        { key: 'monitor', label: 'Monitor', href: './monitor' },
        { key: 'telemetry', label: 'Radio Telemetry', href: './telemetry' },
        { key: 'remote', label: 'Saturn Remote', href: './remote-next' }
      ]
    },
    {
      label: 'Maintenance',
      items: [
        { key: 'update', label: 'G2 Update', href: './update' },
        { key: 'saturngo', label: 'Saturn Go', href: './saturngo' },
        { key: 'fpga', label: 'FPGA Flash', href: './fpga' }
      ]
    },
    {
      label: 'Applications',
      items: [
        { key: 'pihpsdr', label: 'piHPSDR', href: './pihpsdr' },
        { key: 'deskhpsdr', label: 'deskHPSDR', href: './deskhpsdr' }
      ]
    },
    {
      label: 'System',
      items: [
        { key: 'backup', label: 'Backup / Restore', href: './backup' },
        { key: 'tailscale', label: 'Tailscale', href: './tailscale' },
        { key: 'custom', label: 'Custom Scripts', href: './custom' }
      ]
    }
  ];

  function el(tag, attrs, html) {
    var node = document.createElement(tag);
    if (attrs) {
      Object.keys(attrs).forEach(function (k) { node.setAttribute(k, attrs[k]); });
    }
    if (html !== undefined) node.innerHTML = html;
    return node;
  }

  function buildSidebar(active) {
    var sidebar = el('nav', { class: 'saturn-sidebar', id: 'saturn-sidebar', 'aria-label': 'Saturn Go navigation' });
    sidebar.appendChild(el('a', { class: 'saturn-sidebar-brand', href: './' }, 'Saturn Go'));
    NAV_SECTIONS.forEach(function (section) {
      var group = el('div', { class: 'saturn-nav-group' });
      if (section.label) {
        group.appendChild(el('div', { class: 'saturn-nav-group-label' }, section.label));
      }
      section.items.forEach(function (item) {
        var link = el('a', {
          href: item.href,
          class: 'saturn-nav-link' + (item.key === active ? ' active' : ''),
          ...(item.key === active ? { 'aria-current': 'page' } : {})
        }, item.label);
        group.appendChild(link);
      });
      sidebar.appendChild(group);
    });
    return sidebar;
  }

  function applyTheme(light) {
    document.body.classList.toggle('light', light);
    global.dispatchEvent(new CustomEvent('saturn-theme-change', { detail: { light: light } }));
  }

  function wireTheme(topbar) {
    var toggle = el('button', { id: 'theme-toggle', type: 'button', class: 'btn btn-ghost p-2' });
    var icon = el('span', { id: 'theme-icon', 'aria-hidden': 'true' });
    toggle.appendChild(icon);

    function updateControl(isLight) {
      var action = isLight ? 'Use dark theme' : 'Use light theme';
      icon.textContent = isLight ? '\u263e' : '\u2600';
      toggle.setAttribute('title', action);
      toggle.setAttribute('aria-label', action);
    }

    var stored = null;
    try { stored = localStorage.getItem('theme'); } catch (_e) {}
    var light = stored === 'light';
    updateControl(light);
    applyTheme(light);

    toggle.addEventListener('click', function () {
      var isLight = document.body.classList.toggle('light');
      try { localStorage.setItem('theme', isLight ? 'light' : 'dark'); } catch (_e) {}
      updateControl(isLight);
      applyTheme(isLight);
    });

    topbar.appendChild(toggle);
  }

  function wireDrawer(sidebar, trigger) {
    var overlay = el('div', { class: 'saturn-drawer-overlay', id: 'saturn-drawer-overlay' });
    document.body.appendChild(overlay);

    function close() {
      sidebar.classList.remove('open');
      overlay.classList.remove('open');
      document.body.classList.remove('saturn-drawer-open');
      trigger.setAttribute('aria-expanded', 'false');
    }
    function toggleOpen() {
      var opening = !sidebar.classList.contains('open');
      sidebar.classList.toggle('open', opening);
      overlay.classList.toggle('open', opening);
      document.body.classList.toggle('saturn-drawer-open', opening);
      trigger.setAttribute('aria-expanded', opening ? 'true' : 'false');
      if (opening) {
        var firstLink = sidebar.querySelector('a');
        if (firstLink) firstLink.focus();
      }
    }
    overlay.addEventListener('click', close);
    document.addEventListener('keydown', function (evt) {
      if (evt.key === 'Escape') close();
    });
    sidebar.addEventListener('click', function (evt) {
      if (evt.target.closest('a')) close();
    });
    global.addEventListener('resize', function () {
      if (global.innerWidth > 900) close();
    });

    return { toggleOpen: toggleOpen, close: close };
  }

  function mount(opts) {
    opts = opts || {};
    var root = document.getElementById('saturn-shell-mount');
    if (!root) return;

    var sidebar = buildSidebar(opts.active);
    document.body.appendChild(sidebar);
    document.body.classList.add('saturn-shell-active');

    var topbar = el('header', { class: 'saturn-topbar' });

    var hamburger = el('button', { type: 'button', class: 'saturn-hamburger', title: 'Menu', 'aria-label': 'Open navigation menu', 'aria-controls': 'saturn-sidebar', 'aria-expanded': 'false' }, '&#9776;');
    var drawer = wireDrawer(sidebar, hamburger);
    hamburger.addEventListener('click', drawer.toggleOpen);
    topbar.appendChild(hamburger);

    var titleWrap = el('div', { class: 'saturn-topbar-title' });
    titleWrap.appendChild(el('div', { class: 'saturn-topbar-kicker' }, 'Saturn G2'));
    titleWrap.appendChild(el('h1', {}, opts.title || ''));
    if (opts.subtitle) titleWrap.appendChild(el('p', {}, opts.subtitle));
    topbar.appendChild(titleWrap);

    var actions = el('div', { class: 'saturn-topbar-actions' });
    topbar.appendChild(actions);
    wireTheme(actions);

    root.appendChild(topbar);
  }

  global.SaturnShell = { mount: mount };
})(window);
