class Instance {
  constructor() {
    this.last = "constructed";
  }

  ping() {
    this.last = "ping";
  }

  scalar(value) {
    this.last = `scalar:${value}`;
  }

  handle(event) {
    this.last =
      event.tag === "tick" ? `tick:${event.val}` : event.tag;
  }

  getLast() {
    return this.last;
  }
}

export const contract = { Instance };
