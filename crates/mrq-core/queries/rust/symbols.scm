; Symbols of Rust code, for symbol includes. See `mrq_core::symbol::TreeSitterLocator` for
; the captures; where several patterns match a node, the first one counts.

; Doc comments and attributes.
(line_comment outer: (outer_doc_comment_marker)) @doc
(block_comment outer: (outer_doc_comment_marker)) @doc
(attribute_item) @doc

; Items in an impl block, qualified by its type (`Point::new`).
(impl_item
  type: [
    (type_identifier) @parent
    (generic_type type: (type_identifier) @parent)
    (scoped_type_identifier name: (type_identifier) @parent)
  ]
  body: (declaration_list
    [
      (function_item name: (identifier) @name) @definition.function
      (const_item name: (identifier) @name) @definition.constant
      (type_item name: (type_identifier) @name) @definition.type
    ]))

(function_item name: (identifier) @name) @definition.function
(function_signature_item name: (identifier) @name) @definition.function
(struct_item name: (type_identifier) @name) @definition.struct
(union_item name: (type_identifier) @name) @definition.struct
(enum_item name: (type_identifier) @name) @definition.enum
(trait_item name: (type_identifier) @name) @definition.trait
(type_item name: (type_identifier) @name) @definition.type
(mod_item name: (identifier) @name) @definition.module
(const_item name: (identifier) @name) @definition.constant
(static_item name: (identifier) @name) @definition.constant
(macro_definition name: (identifier) @name) @definition.macro
