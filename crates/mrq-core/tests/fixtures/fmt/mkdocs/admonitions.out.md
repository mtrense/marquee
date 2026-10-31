# Admonitions

!!! note
    A plain note with *emphasis*.

!!! warning "Careful now"
    First paragraph
    continued.

    Second paragraph.

??? tip "Folded"
    Indented by a tab.

???+ info inline end
    Open, with extra classes.

!!! danger ""
    No title bar.

!!! abstract

!!! example "Directly after"
    ```
    Indented code in the body.
    ```

after the body

!!! note "Outer"
    Outer text.

    !!! warning "Inner"
        - one
        - two

    Back in outer.

- Item

  !!! note
      In a list item.

> !!! quote "In a quote"
>     Quoted body.
>
>     More quoted body.

Not admonitions:

\!!! note
stays text.
