export function openAccountsDialog(provider?: string) {
  window.dispatchEvent(
    new CustomEvent("overmind:open-accounts", { detail: { provider } }),
  );
}

export function notifyAccountActivated(provider: string) {
  window.dispatchEvent(
    new CustomEvent("overmind:account-activated", { detail: { provider } }),
  );
}
