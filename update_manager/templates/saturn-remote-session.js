(function () {
  function parseTciText(text) {
    return String(text)
      .split(";")
      .map((part) => part.trim())
      .filter((part) => part.length > 0)
      .map((raw) => {
        const colon = raw.indexOf(":");
        if (colon < 0) {
          return { name: raw.trim().toLowerCase(), args: [], raw };
        }
        return {
          name: raw.slice(0, colon).trim().toLowerCase(),
          args: raw.slice(colon + 1).split(",").map((arg) => arg.trim()),
          raw,
        };
      });
  }

  function classifySocketMessage(data) {
    if (typeof data === "string") {
      const commands = parseTciText(data);
      return {
        kind: "text",
        text: data,
        commands,
        ready: commands.some((command) => command.name === "ready"),
      };
    }

    if (data instanceof ArrayBuffer) {
      if (data.byteLength < 64) {
        return { kind: "ignored", accepted: false };
      }
      const view = new DataView(data);
      return {
        kind: "binary",
        accepted: true,
        buffer: data,
        frameType: view.getUint32(24, true) === 1 ? "audio" : "iq",
      };
    }

    return { kind: "ignored", accepted: false };
  }

  window.SaturnRemoteSession = {
    parseTciText,
    classifySocketMessage,
  };
})();
