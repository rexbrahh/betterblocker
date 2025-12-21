(() => {
  // src/cs/bootstrap.ts
  (() => {
    if (document.documentElement.dataset.bbInjected) {
      return;
    }
    document.documentElement.dataset.bbInjected = "1";
  })();
})();
