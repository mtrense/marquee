# Code

<!-- mrq:include path="code/hello.rs" lang=rust -->
<!-- mrq:end -->

Lines 5 to 7, dedented by default:

<!-- mrq:include path="code/hello.rs" lang=rust lines=5-7 -->
<!-- mrq:end -->

<!-- mrq:include path="code/hello.rs" lang=rust lines=5-7 dedent=false -->
<!-- mrq:end -->

<!-- mrq:include path="code/hello.rs" lang=rust lines=-1 -->
<!-- mrq:end -->

<!-- mrq:include path="code/hello.rs" lang=rust lines=6- -->
<!-- mrq:end -->

<!-- mrq:include path="code/hello.rs" lang=rust lines=1-100 -->
<!-- mrq:end -->

Markdown with fences, as code:

<!-- mrq:include path="code/fences.md" lang=markdown -->
<!-- mrq:end -->

> In a quote:
>
> <!-- mrq:include path="code/hello.rs" lang=rust lines=6 -->
> <!-- mrq:end -->

- In a list:

  <!-- mrq:include path="code/hello.rs" lang=rust lines=3-5 -->
  <!-- mrq:end -->

This file, as code, is not a cycle:

<!-- mrq:include path="code.md" lang=markdown lines=-3 -->
<!-- mrq:end -->
