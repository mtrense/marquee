// An `overrides.pdf` template for the tests: a header from a file next to it, and the
// title from `sys.inputs`.
#let template(body) = {
  set page(header: align(right, read("header.txt").trim()))
  [Title: #sys.inputs.at("title", default: "none")]
  body
}
