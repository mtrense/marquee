"""Greeters."""

import functools


# A comment, not a decorator.
@functools.total_ordering
class Greeter:
    """Greets people."""

    def __init__(self, name):
        self.name = name

    @property
    def greeting(self):
        """The greeting."""
        return f"Hello, {self.name}!"

    def greet(self):
        print(self.greeting)


class Shouter(Greeter):
    def greet(self):
        print(self.greeting.upper())


@functools.cache
def main():
    Greeter("world").greet()
