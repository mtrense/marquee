; Symbols of Python code, for symbol includes. See `mrq_core::symbol::TreeSitterLocator` for
; the captures; where several patterns match a node, the first one counts. Docstrings are
; part of the body.

; Decorators.
(decorator) @doc

(function_definition name: (identifier) @name) @definition.function
(class_definition name: (identifier) @name) @definition.class
