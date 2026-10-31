; Symbols of TypeScript (and TSX) code, for symbol includes, on top of the JavaScript
; patterns in `../javascript/symbols.scm`. See `mrq_core::symbol::TreeSitterLocator` for the
; captures; where several patterns match a node, the first one counts.

(abstract_class_declaration name: (_) @name) @definition.class
(abstract_method_signature name: (_) @name) @definition.function
(method_signature name: (_) @name) @definition.function
(function_signature name: (_) @name) @definition.function
(interface_declaration name: (_) @name) @definition.interface
(type_alias_declaration name: (_) @name) @definition.type
(enum_declaration name: (_) @name) @definition.enum
(internal_module name: (_) @name) @definition.module
