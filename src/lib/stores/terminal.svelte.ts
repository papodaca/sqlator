let showTerminal = $state(false);
let terminalId = $state<string | null>(null);

export const terminalStore = {
  get show() {
    return showTerminal;
  },
  get terminalId() {
    return terminalId;
  },
  toggle() {
    showTerminal = !showTerminal;
  },
  open() {
    showTerminal = true;
  },
  close() {
    showTerminal = false;
  },
  setTerminalId(id: string | null) {
    terminalId = id;
  },
};
