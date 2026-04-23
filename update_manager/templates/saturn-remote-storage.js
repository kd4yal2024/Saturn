(function () {
  const STORAGE_KEY = "saturn.remote.settings";

  function cloneSettings(settings) {
    return JSON.parse(JSON.stringify(settings));
  }

  function load() {
    try {
      const raw = window.localStorage.getItem(STORAGE_KEY);
      if (!raw) {
        return null;
      }
      const parsed = JSON.parse(raw);
      return parsed && typeof parsed === "object" ? parsed : null;
    } catch (error) {
      console.warn("Saturn Remote runtime storage load failed", error);
      return null;
    }
  }

  function save(settings) {
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(cloneSettings(settings)));
      return true;
    } catch (error) {
      console.warn("Saturn Remote runtime storage save failed", error);
      return false;
    }
  }

  window.SaturnRemoteStorage = { load, save };
})();
