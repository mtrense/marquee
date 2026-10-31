# Python

Docstrings belong to the body; decorators come with `with_docs`:

```python
class Greeter:
    """Greets people."""
```

```python
@functools.total_ordering
class Greeter:
    """Greets people."""
```

```python
def greeting(self):
    """The greeting."""
    return f"Hello, {self.name}!"
```

```python
@property
def greeting(self):
    """The greeting."""
    return f"Hello, {self.name}!"
```

```python
def greet(self):
    print(self.greeting.upper())
```

```py
@functools.cache
def main():
    Greeter("world").greet()
```
