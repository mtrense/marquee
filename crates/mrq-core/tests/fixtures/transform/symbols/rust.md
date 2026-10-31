# Rust

The fence language comes from the file extension:

<!-- mrq:include path="code/shapes.rs" function=main -->
<!-- mrq:end -->

<!-- mrq:include path="code/shapes.rs" struct=Point -->
<!-- mrq:end -->

<!-- mrq:include path="code/shapes.rs" struct=Point with_docs=true -->
<!-- mrq:end -->

A method, dedented, and `lines` counted from the symbol:

<!-- mrq:include path="code/shapes.rs" function="Point::norm" -->
<!-- mrq:end -->

<!-- mrq:include path="code/shapes.rs" lang=rs function="Point::new" lines=2 -->
<!-- mrq:end -->

<!-- mrq:include path="code/shapes.rs" function="Point::new" dedent=false with_docs=true -->
<!-- mrq:end -->

Any kind of symbol, trait methods, trait impls and modules:

<!-- mrq:include path="code/shapes.rs" symbol=Shape with_docs=true -->
<!-- mrq:end -->

<!-- mrq:include path="code/shapes.rs" symbol="Area::area" with_docs=true -->
<!-- mrq:end -->

<!-- mrq:include path="code/shapes.rs" function=fmt -->
<!-- mrq:end -->

<!-- mrq:include path="code/shapes.rs" function="util::clamp" with_docs=true -->
<!-- mrq:end -->

<!-- mrq:include path="code/shapes.rs" symbol=util -->
<!-- mrq:end -->

<!-- mrq:include path="code/shapes.rs" symbol=ORIGIN with_docs=true -->
<!-- mrq:end -->

<!-- mrq:include path="code/shapes.rs" function=main with_docs=true -->
<!-- mrq:end -->
