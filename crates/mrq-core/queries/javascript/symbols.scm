; Symbols of JavaScript code, for symbol includes; TypeScript adds its own patterns to these.
; See `mrq_core::symbol::TreeSitterLocator` for the captures; where several patterns match a
; node, the first one counts.

; JSDoc comments and decorators.
((comment) @doc (#match? @doc "^/\\*\\*"))
(decorator) @doc

; `export` belongs to the declaration.
(export_statement) @wrapper

(function_declaration name: (_) @name) @definition.function
(generator_function_declaration name: (_) @name) @definition.function
(method_definition name: (_) @name) @definition.function
(class_declaration name: (_) @name) @definition.class

; Functions assigned to a variable: `const shout = (text) => …`.
(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function_expression) (generator_function)])) @definition.function
(variable_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function_expression) (generator_function)])) @definition.function
