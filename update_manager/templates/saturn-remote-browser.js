(function () {
  function createRuntime(deps) {
    const {
      state,
      constants,
      getSessionApi,
      getTciApi,
      getTransportApi,
      shiftDisplayForCenterChange,
      resetDisplayHistory,
      applyRxVolumeGain,
      stopMicCapture,
      recoverRxAfterMox,
      startRxAudio,
      logEvent,
      updateUi,
      flushRxAudioQueue,
      scheduleAudioPlayback,
    } = deps;

    function parseTciCommands(text) {
      return getSessionApi().parseTciText(text);
    }

    function syncTciUiSideEffects(previous, next, events) {
      if (events.ready) {
        logEvent("Bridge reported ready");
      }
      if (events.displayCenterHzChanged) {
        shiftDisplayForCenterChange(next.dds);
      }
      if (events.sampleRateChanged) {
        resetDisplayHistory();
      }

      Object.assign(state, next);

      if (previous.filterLow !== state.filterLow) {
        document.getElementById("filter-low").value = `${state.filterLow}`;
      }
      if (previous.filterHigh !== state.filterHigh) {
        document.getElementById("filter-high").value = `${state.filterHigh}`;
      }
      if (previous.txFilterLow !== state.txFilterLow) {
        const txFlEl = document.getElementById("tx-filter-low");
        if (txFlEl) txFlEl.value = `${state.txFilterLow}`;
      }
      if (previous.txFilterHigh !== state.txFilterHigh) {
        const txFhEl = document.getElementById("tx-filter-high");
        if (txFhEl) txFhEl.value = `${state.txFilterHigh}`;
      }

      for (let band = 1; band <= 10; band += 1) {
        if (previous.rxEqBands?.[band] !== state.rxEqBands?.[band]) {
          const slider = document.getElementById(`rx-eq-band-${band}`);
          const valEl = document.getElementById(`rx-eq-val-${band}`);
          if (slider) slider.value = `${Math.round(state.rxEqBands[band])}`;
          if (valEl) valEl.textContent = `${Math.round(state.rxEqBands[band])}`;
        }
        if (previous.txEqBands?.[band] !== state.txEqBands?.[band]) {
          const slider = document.getElementById(`tx-eq-band-${band}`);
          const valEl = document.getElementById(`tx-eq-val-${band}`);
          if (slider) slider.value = `${Math.round(state.txEqBands[band])}`;
          if (valEl) valEl.textContent = `${Math.round(state.txEqBands[band])}`;
        }
        if (previous.cfcBands?.[band] !== state.cfcBands?.[band]) {
          const slider = document.getElementById(`cfc-band-${band}`);
          const valEl = document.getElementById(`cfc-val-${band}`);
          if (slider) slider.value = `${state.cfcBands[band]}`;
          if (valEl) valEl.textContent = `${state.cfcBands[band].toFixed(1)}`;
        }
      }

      if (events.rxVolumeChanged) {
        applyRxVolumeGain();
      }
      if (events.txReleased) {
        stopMicCapture();
        recoverRxAfterMox();
        if (state.connected && !state.audioStreaming) {
          void startRxAudio();
        }
      }
    }

    function handleTciText(text, parsedCommands) {
      const commands = Array.isArray(parsedCommands) ? parsedCommands : parseTciCommands(text);
      const previousState = {
        ...state,
        rxEqBands: Array.isArray(state.rxEqBands) ? state.rxEqBands.slice() : [],
        txEqBands: Array.isArray(state.txEqBands) ? state.txEqBands.slice() : [],
        cfcBands: Array.isArray(state.cfcBands) ? state.cfcBands.slice() : [],
      };
      const applied = getTciApi().applyCommands(commands, state);
      syncTciUiSideEffects(previousState, applied.state, applied.events);
      updateUi();
    }

    function handleBinaryFrame(buffer) {
      if (!(buffer instanceof ArrayBuffer) || buffer.byteLength < 64) {
        return;
      }
      const classified = getTransportApi().classifyFrame(buffer);
      if (classified === "audio") {
        handleAudioFrame(buffer);
        return;
      }
      handleIqFrame(buffer);
    }

    function fftSizeForSamples(value, zoom) {
      return getTransportApi().fftSizeForSamples(
        value,
        zoom,
        constants.DISPLAY_BASE_FFT_SIZE,
        constants.DISPLAY_MAX_FFT_SIZE
      );
    }

    function buildRenderIqWindow() {
      return getTransportApi().buildRenderIqWindow({
        iqPackets: state.iqPackets,
        displayZoom: state.displayZoom,
        maxFftSize: constants.DISPLAY_MAX_FFT_SIZE,
        baseFftSize: constants.DISPLAY_BASE_FFT_SIZE,
      });
    }

    function handleIqFrame(buffer) {
      const transportApi = getTransportApi();
      const applied = transportApi.applyIqFrame({
        sampleRate: state.sampleRate,
        displayZoom: state.displayZoom,
        iqPackets: state.iqPackets,
        packetHistoryBase: constants.DISPLAY_PACKET_HISTORY_BASE,
        packetHistoryMax: constants.DISPLAY_PACKET_HISTORY_MAX,
        maxFftSize: constants.DISPLAY_MAX_FFT_SIZE,
        baseFftSize: constants.DISPLAY_BASE_FFT_SIZE,
        moxRequested: state.moxRequested,
        txEnabled: state.txEnabled,
        lastFrameAt: state.lastFrameAt,
        frameCounter: state.frameCounter,
        displayCaption: state.displayCaption,
      }, transportApi.decodeIqFrame(buffer), performance.now());
      if (!applied.accepted) {
        return;
      }
      if (applied.events.sampleRateChanged) {
        resetDisplayHistory();
      }
      state.sampleRate = applied.state.sampleRate;
      state.iqPackets = applied.state.iqPackets;
      state.lastFrameAt = applied.state.lastFrameAt;
      state.frameCounter = applied.state.frameCounter;
      state.displayCaption = applied.state.displayCaption;
      if (applied.events.receivedIqLogMessage) {
        logEvent(applied.events.receivedIqLogMessage);
      }
      updateUi();
    }

    function writeRxAudioFrameToSab(left, right) {
      if (!state.rxRingF32 || !state.rxCtrlU32) {
        return false;
      }
      const ring = state.rxRingF32;
      const ctrl = state.rxCtrlU32;
      const cap = constants.RX_RING_FRAMES;
      const frames = Math.min(left.length, cap - 1);
      let rd = Atomics.load(ctrl, constants.CTRL_READ_IDX);
      let wr = Atomics.load(ctrl, constants.CTRL_WRITE_IDX);
      const used = wr >= rd ? wr - rd : cap - rd + wr;
      let free = cap - used - 1;
      if (free < frames) {
        const advance = frames - free;
        rd = (rd + advance) % cap;
        Atomics.store(ctrl, constants.CTRL_READ_IDX, rd);
        state.rxWorkletDrops += 1;
        if (state.rxWorkletDrops === 1 || state.rxWorkletDrops % 25 === 0) {
          logEvent(`RX worklet ring resynced ${state.rxWorkletDrops} time(s)`);
        }
      }
      for (let i = 0; i < frames; i += 1) {
        const idx = wr * 2;
        ring[idx] = left[i];
        ring[idx + 1] = right[i];
        wr = (wr + 1) % cap;
      }
      Atomics.store(ctrl, constants.CTRL_WRITE_IDX, wr);
      return true;
    }

    function handleAudioFrame(buffer) {
      const transportApi = getTransportApi();
      const decoded = transportApi.decodeAudioFrame(buffer);
      const applied = transportApi.applyAudioFrame({
        audioStreaming: state.audioStreaming,
        audioSampleRate: state.audioSampleRate,
        audioFramesPlayed: state.audioFramesPlayed,
        moxRequested: state.moxRequested,
        txEnabled: state.txEnabled,
      }, decoded);
      if (!applied.accepted) {
        if (!state.audioCtx || !state.audioStreaming) {
          return;
        }
        if (state.moxRequested || state.txEnabled) {
          flushRxAudioQueue();
          return;
        }
      } else {
        state.audioSampleRate = applied.state.audioSampleRate;
        state.audioFramesPlayed = applied.state.audioFramesPlayed;
      }
      if (!decoded) {
        return;
      }
      if (!state.audioCtx || !state.audioStreaming) {
        return;
      }
      if (state.moxRequested || state.txEnabled) {
        flushRxAudioQueue();
        return;
      }
      if (state.audioCtx.state === "suspended") {
        void state.audioCtx.resume();
      }
      if (state.audioWorkletMode === "sab") {
        if (writeRxAudioFrameToSab(decoded.left, decoded.right)) {
          state.audioFramesPlayed += 1;
        }
        return;
      }
      if (state.audioWorkletMode === "msg" && state.rxWorkletNode) {
        state.rxWorkletNode.port.postMessage(
          { type: "audio", left: decoded.left, right: decoded.right },
          [decoded.left.buffer, decoded.right.buffer]
        );
        state.audioFramesPlayed += 1;
        return;
      }

      const audioBuffer = state.audioCtx.createBuffer(2, decoded.frames, decoded.header.sampleRate);
      audioBuffer.copyToChannel(decoded.left, 0);
      audioBuffer.copyToChannel(decoded.right, 1);
      scheduleAudioPlayback(audioBuffer);
    }

    return {
      parseTciCommands,
      handleTciText,
      handleBinaryFrame,
      fftSizeForSamples,
      buildRenderIqWindow,
      handleIqFrame,
      handleAudioFrame,
    };
  }

  window.SaturnRemoteBrowserRuntime = { create: createRuntime };
})();
