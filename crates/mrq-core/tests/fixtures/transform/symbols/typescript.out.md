# TypeScript

```typescript
/** A point in the plane. */
export interface Point {
  x: number;
  y: number;
}
```

```typescript
export type Pair = [Point, Point];
```

```typescript
enum Color {
  Red,
  Green,
}
```

```typescript
export abstract class Shape {
  abstract area(): number;
}
```

```typescript
/** A shape. */
@sealed
export abstract class Shape {
  abstract area(): number;
}
```

```typescript
abstract area(): number;
```

```typescript
area(): number {
  return Math.PI * this.radius ** 2;
}
```

```typescript
@memo
area(): number {
  return Math.PI * this.radius ** 2;
}
```

```typescript
export function origin(): Point {
  return { x: 0, y: 0 };
}
```
