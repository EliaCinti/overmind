export function openAccountsDialog(provider?: string) {
  window.dispatchEvent(
    new CustomEvent("overmind:open-accounts", { detail: { provider } }),
  );
}
