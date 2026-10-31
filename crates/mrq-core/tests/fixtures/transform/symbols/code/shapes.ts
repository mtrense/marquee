/** A point in the plane. */
export interface Point {
  x: number;
  y: number;
}

export type Pair = [Point, Point];

enum Color {
  Red,
  Green,
}

/** A shape. */
@sealed
export abstract class Shape {
  abstract area(): number;
}

export class Circle extends Shape {
  constructor(private radius: number) {
    super();
  }

  @memo
  area(): number {
    return Math.PI * this.radius ** 2;
  }
}

export function origin(): Point {
  return { x: 0, y: 0 };
}
