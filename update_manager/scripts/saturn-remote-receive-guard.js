// Receive-only guard for the update process's headless validation session.
//
// The validator (saturn-remote-validate.py) injects this file into the temporary browser BEFORE any page script runs, whatever release the
// page belongs to. That matters after a rollback: the release being restored may predate the page's own validation mode, and its page would
// otherwise behave like a normal operator and retune the radio. The text between the RECEIVE_ONLY_GUARD markers is byte-identical to the
// copy inside saturn-remote-next.html; tests enforce that, and enforce that it allows nothing the Bridge would refuse a viewer.
(function () {
    /* RECEIVE_ONLY_GUARD_BEGIN */
    // The receive-only guard. A validation session (the update process's temporary headless browser, which proves after an install or a
    // rollback that reception really works) may negotiate its own audio and display stream and listen. It must change NOTHING about the
    // radio: no tuning, mode, rate, volume, filter or TX setting, and no saved profile or setting. Independent layers hold that line:
    //   1. the session is opened as a VIEWER, and the Bridge itself discards every command a viewer may not send (viewer_tci_command_allowed);
    //   2. this deny-by-default allowlist sits on WebSocket.send, so a command added to some other code path later is dropped as well,
    //      binary uplink frames are never sent, and nothing but GET/HEAD requests leaves the page;
    //   3. the validator watches the frames that really leave the browser from outside the page and fails on anything not allowed.
    // This text exists twice and must stay byte-identical: here in the page, and in saturn-remote-receive-guard.js, which the validator
    // injects before any page script runs. That covers the release that is being rolled back TO, whose page predates this mode.
    // The allowlist must stay inside the Bridge's viewer list; a test compares them.
    function createReceiveOnlyGuardModule() {
      const ALLOWED_COMMANDS = Object.freeze([
        "audio_codec", "audio_gain", "audio_samplerate", "audio_start", "audio_stop", "audio_stream_channels",
        "audio_stream_samples", "audio_stream_sample_type", "iq_start", "iq_stop", "saturn_display", "saturn_display_ack",
        "saturn_ping", "session_open",
      ]);
      const allowed = new Set(ALLOWED_COMMANDS);
      function requested(search) {
        try {
          return new URLSearchParams(search || "").get("validation") === "receive";
        } catch (_) {
          return false;
        }
      }
      function filterText(text) {
        const sent = [];
        const blocked = [];
        for (const segment of String(text).split(";")) {
          const command = segment.trim();
          if (!command) continue;
          const name = command.split(":", 1)[0].trim().toLowerCase();
          if (!allowed.has(name)) {
            blocked.push(command);
          } else if (name === "session_open") {
            // The Bridge enforces what a viewer may do, so the session is always opened as one, whatever role the page asked for.
            const args = command.slice(command.indexOf(":") + 1).split(",");
            if (args.length >= 2) args[args.length - 1] = "viewer"; else args.push("viewer");
            sent.push(`session_open:${args.join(",")}`);
          } else {
            sent.push(command);
          }
        }
        return { text: sent.map((command) => `${command};`).join(""), blocked };
      }
      function newRecorder() {
        const recorder = {
          blockedCounts: {},
          blockedExamples: [],
          violations: [],
          errors: [],
          blocked(command) {
            const name = String(command).split(":", 1)[0].trim().toLowerCase();
            recorder.blockedCounts[name] = (recorder.blockedCounts[name] || 0) + 1;
            if (recorder.blockedExamples.length < 40) recorder.blockedExamples.push(String(command).slice(0, 120));
          },
          violation(kind, detail) {
            if (recorder.violations.length < 40) recorder.violations.push({ kind, detail: String(detail).slice(0, 160) });
          },
          error(kind, detail) {
            if (recorder.errors.length < 40) recorder.errors.push({ kind, detail: String(detail).slice(0, 240) });
          },
        };
        return recorder;
      }
      function recorderFor(win) {
        if (!win.saturnReceiveGuardRecorder) win.saturnReceiveGuardRecorder = newRecorder();
        return win.saturnReceiveGuardRecorder;
      }
      function install(win) {
        const recorder = recorderFor(win);
        const WS = win.WebSocket;
        if (WS && WS.prototype && typeof WS.prototype.send === "function" && !WS.prototype.send.receiveOnlyGuard) {
          const nativeSend = WS.prototype.send;
          const guardedSend = function send(data) {
            if (typeof data === "string") {
              const filtered = filterText(data);
              for (const command of filtered.blocked) recorder.blocked(command);
              if (filtered.text === "") return undefined;
              return nativeSend.call(this, filtered.text);
            }
            // A receive-only session has no uplink media, so no binary frame is ever sent.
            recorder.violation("binary-send", `${data && data.byteLength !== undefined ? data.byteLength : "?"} bytes`);
            return undefined;
          };
          guardedSend.receiveOnlyGuard = true;
          WS.prototype.send = guardedSend;
        }
        if (typeof win.fetch === "function" && !win.fetch.receiveOnlyGuard) {
          const nativeFetch = win.fetch;
          const guardedFetch = function fetch(input, init) {
            const method = String((init && init.method) || (input && input.method) || "GET").toUpperCase();
            if (method !== "GET" && method !== "HEAD") {
              recorder.violation(`fetch-${method}`, (input && input.url) || input);
              return Promise.reject(new TypeError(`receive-only validation session: ${method} is not permitted`));
            }
            return nativeFetch.call(this, input, init);
          };
          guardedFetch.receiveOnlyGuard = true;
          win.fetch = guardedFetch;
        }
        if (win.navigator && typeof win.navigator.sendBeacon === "function" && !win.navigator.sendBeacon.receiveOnlyGuard) {
          const guardedBeacon = function sendBeacon(url) {
            recorder.violation("beacon", url);
            return false;
          };
          guardedBeacon.receiveOnlyGuard = true;
          win.navigator.sendBeacon = guardedBeacon;
        }
        const XHR = win.XMLHttpRequest;
        if (XHR && XHR.prototype && typeof XHR.prototype.open === "function" && !XHR.prototype.open.receiveOnlyGuard) {
          const nativeOpen = XHR.prototype.open;
          const guardedOpen = function open(method, ...rest) {
            const upper = String(method).toUpperCase();
            if (upper !== "GET" && upper !== "HEAD") {
              recorder.violation(`xhr-${upper}`, rest[0]);
              throw new Error(`receive-only validation session: ${upper} is not permitted`);
            }
            return nativeOpen.call(this, method, ...rest);
          };
          guardedOpen.receiveOnlyGuard = true;
          XHR.prototype.open = guardedOpen;
        }
        if (typeof win.addEventListener === "function" && !win.saturnReceiveGuardListening) {
          win.saturnReceiveGuardListening = true;
          win.addEventListener("error", (event) => recorder.error("uncaught", (event && event.message) || "error"));
          win.addEventListener("unhandledrejection", (event) => {
            const reason = event && event.reason;
            recorder.error("unhandled-rejection", (reason && reason.message) || reason || "rejection");
          });
        }
        return recorder;
      }
      return { ALLOWED_COMMANDS, requested, filterText, newRecorder, recorderFor, install };
    }
    /* RECEIVE_ONLY_GUARD_END */
    createReceiveOnlyGuardModule().install(window);
})();
