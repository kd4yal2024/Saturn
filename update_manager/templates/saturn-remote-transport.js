(function () {
  const TCI_FRAME_HEADER_BYTES = 64;
  const TCI_STREAM_TYPE_AUDIO_LEFT = 1;

  function parseFrameHeader(buffer) {
    if (!(buffer instanceof ArrayBuffer) || buffer.byteLength < TCI_FRAME_HEADER_BYTES) {
      return null;
    }
    const view = new DataView(buffer);
    return {
      sampleRate: view.getUint32(4, true),
      floatCount: view.getUint32(20, true),
      frameType: view.getUint32(24, true),
      channels: Math.max(1, view.getUint32(28, true) || 2),
    };
  }

  function payloadFloats(buffer, floatCount) {
    const expectedBytes = TCI_FRAME_HEADER_BYTES + floatCount * Float32Array.BYTES_PER_ELEMENT;
    if (buffer.byteLength < expectedBytes) return null;
    return new Float32Array(buffer, TCI_FRAME_HEADER_BYTES, floatCount);
  }

  function classifyFrame(buffer) {
    const header = parseFrameHeader(buffer);
    if (!header) return null;
    return header.frameType === TCI_STREAM_TYPE_AUDIO_LEFT ? "audio" : "iq";
  }

  function decodeIqFrame(buffer) {
    const header = parseFrameHeader(buffer);
    if (!header || !header.floatCount) return null;
    const iq = payloadFloats(buffer, header.floatCount);
    if (!iq) return null;
    return {
      kind: "iq",
      header,
      iq,
      samplePairs: Math.floor(iq.length / 2),
    };
  }

  function decodeAudioFrame(buffer) {
    const header = parseFrameHeader(buffer);
    if (!header || !header.floatCount) return null;
    const floats = payloadFloats(buffer, header.floatCount);
    if (!floats) return null;

    const channels = Math.max(1, header.channels);
    const frames = Math.floor(floats.length / channels);
    if (frames < 1) return null;

    const left = new Float32Array(frames);
    const right = new Float32Array(frames);

    let mirroredMono = channels < 2;
    if (!mirroredMono && channels >= 2) {
      mirroredMono = true;
      const probeFrames = Math.min(frames, 128);
      for (let i = 0; i < probeFrames; i += 1) {
        if (Math.abs(floats[i * channels + 1] ?? 0) > 1e-6) {
          mirroredMono = false;
          break;
        }
      }
    }

    for (let i = 0; i < frames; i += 1) {
      const leftSample = floats[i * channels] ?? 0;
      const rightSample = mirroredMono
        ? leftSample
        : (channels > 1 ? (floats[i * channels + 1] ?? leftSample) : leftSample);
      left[i] = leftSample;
      right[i] = rightSample;
    }

    return {
      kind: "audio",
      header,
      frames,
      left,
      right,
      mirroredMono,
    };
  }

  function decodeFrame(buffer) {
    const kind = classifyFrame(buffer);
    if (kind === "audio") return decodeAudioFrame(buffer);
    if (kind === "iq") return decodeIqFrame(buffer);
    return null;
  }

  function applyIqFrame(current, decoded, nowMs) {
    if (!decoded || decoded.kind !== "iq") {
      return { state: current, accepted: false, events: { sampleRateChanged: false, receivedIqLogMessage: null } };
    }
    if (current.moxRequested || current.txEnabled) {
      return { state: current, accepted: false, events: { sampleRateChanged: false, receivedIqLogMessage: null } };
    }

    const nextPackets = current.iqPackets.slice();
    nextPackets.push(new Float32Array(decoded.iq));
    const historyLimit = Math.min(
      current.packetHistoryMax,
      current.packetHistoryBase * Math.max(1, current.displayZoom),
    );
    while (nextPackets.length > historyLimit) {
      nextPackets.shift();
    }

    const sampleRateChanged = decoded.header.sampleRate !== current.sampleRate;
    const receivedIqLogMessage = nowMs - current.lastFrameAt > 2500
      ? `Received IQ frame: ${decoded.samplePairs} complex samples @ ${Math.round(decoded.header.sampleRate / 1000)} kHz`
      : null;

    return {
      accepted: true,
      state: {
        ...current,
        sampleRate: decoded.header.sampleRate,
        iqPackets: nextPackets,
        lastFrameAt: nowMs,
        frameCounter: current.frameCounter + 1,
        displayCaption: `Live IQ frame: ${decoded.samplePairs} complex samples at ${Math.round(decoded.header.sampleRate / 1000)} kHz`,
      },
      events: {
        sampleRateChanged,
        receivedIqLogMessage,
      },
    };
  }

  function applyAudioFrame(current, decoded) {
    if (!decoded || decoded.kind !== "audio") {
      return { state: current, accepted: false };
    }
    if (!current.audioStreaming || current.moxRequested || current.txEnabled) {
      return { state: current, accepted: false };
    }
    return {
      accepted: true,
      state: {
        ...current,
        audioSampleRate: decoded.header.sampleRate,
        audioFramesPlayed: current.audioFramesPlayed,
      },
    };
  }

  function fftSizeForSamples(value, zoom, baseFftSize, maxFftSize) {
    const target = Math.min(maxFftSize, baseFftSize * Math.max(1, zoom || 1));
    if (value >= target) return target;
    let size = 32;
    while ((size << 1) <= value && (size << 1) <= target) {
      size <<= 1;
    }
    return Math.max(32, size);
  }

  function buildRenderIqWindow(current) {
    if (!current || !Array.isArray(current.iqPackets) || current.iqPackets.length === 0) {
      return null;
    }

    const desiredComplexSamples = Math.min(
      current.maxFftSize,
      current.baseFftSize * Math.max(1, current.displayZoom),
    );
    const selected = [];
    let totalFloats = 0;

    for (let i = current.iqPackets.length - 1; i >= 0; i -= 1) {
      const packet = current.iqPackets[i];
      if (!packet) continue;
      selected.push(packet);
      totalFloats += packet.length;
      if ((totalFloats / 2) >= desiredComplexSamples) break;
    }

    selected.reverse();
    const combined = new Float32Array(totalFloats);
    let offset = 0;
    for (const packet of selected) {
      combined.set(packet, offset);
      offset += packet.length;
    }
    return combined;
  }

  window.SaturnRemoteTransport = {
    classifyFrame,
    decodeFrame,
    decodeIqFrame,
    decodeAudioFrame,
    applyIqFrame,
    applyAudioFrame,
    fftSizeForSamples,
    buildRenderIqWindow,
  };
})();
