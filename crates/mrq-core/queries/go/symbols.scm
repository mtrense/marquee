; Symbols of Go code, for symbol includes. See `mrq_core::symbol::TreeSitterLocator` for the
; captures; where several patterns match a node, the first one counts.

; Doc comments: Go's are plain comments right before the declaration.
(comment) @doc

; `type` and `const` belong to a declaration of one type or constant.
(type_declaration) @wrapper
(const_declaration) @wrapper

; Methods, qualified by their receiver's type (`Server.Start`).
(method_declaration
  receiver: (parameter_list
    (parameter_declaration
      type: [
        (type_identifier) @parent
        (pointer_type (type_identifier) @parent)
        (generic_type type: (type_identifier) @parent)
        (pointer_type (generic_type type: (type_identifier) @parent))
      ]))
  name: (field_identifier) @name) @definition.function

(function_declaration name: (identifier) @name) @definition.function
(type_spec name: (type_identifier) @name type: (struct_type)) @definition.struct
(type_spec name: (type_identifier) @name type: (interface_type)) @definition.interface
(type_spec name: (type_identifier) @name) @definition.type
(type_alias name: (type_identifier) @name) @definition.type
(const_spec name: (identifier) @name) @definition.constant
