// Greeters.

/**
 * Greets people.
 */
export class Greeter {
  constructor(name) {
    this.name = name;
  }

  /** Greets. */
  greet() {
    console.log(`Hello, ${this.name}!`);
  }
}

// Not JSDoc.
export function main() {
  new Greeter("world").greet();
}

const shout = (text) => text.toUpperCase();

function* count() {
  yield 1;
}
