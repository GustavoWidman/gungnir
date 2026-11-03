#import "@preview/charged-ieee:0.1.4": ieee

#set page(footer: context align(center, text(size: 8pt, fill: rgb("555555"))[#counter(page).display()]))

#show: ieee.with(
  title: [Gungnir: Reasoning Effort, Programming Language, and Vendor Affinity in Coding Agents],
  authors: (
    (name: "Gustavo Wagon Widman", organization: [Instituto de Tecnologia e Liderança (Inteli)], location: [São Paulo, Brazil]),
  ),
  abstract: [
    Draft. This study measures how the reasoning-effort setting of current coding agents interacts with the programming language of the task. Each system under test is a vendor harness, a model, and an effort level. Every system solves the same language-neutral scenarios in eleven languages, from Python to Zig and x86-64 assembly. We record task success against hidden tests, the runtime of the produced program relative to a same-language reference, its complexity, and the tokens and list-price cost of the run.
  ],
  index-terms: ([coding agents], [reasoning effort], [programming languages], [benchmarking], [reproducibility]),
  bibliography: none,
)

#set text(font: "TeX Gyre Termes", lang: "en")
#show raw: set text(font: "TeX Gyre Cursor")

= Introduction

Research questions. *RQ1:* how does reasoning effort change success, cost, and the runtime of the produced code, and does that change depend on the language? *RQ2:* do vendor stacks have language affinities once language difficulty and overall strength are accounted for? *RQ3:* how much does the harness alone move the results?

= Related Work

Agents now write working programs in many languages, yet language still shapes cost and attainable quality @acher2026chess @wu2026tokenmaxxing. Harness choice changes measured results for the same model @merrill2026terminalbench @arjmandi2026harness, and rankings of model-plus-harness systems are far more reliable than rankings of bare models @hardy2026reliability.

= Method

= Results

= Threats to Validity

= Conclusion

#show bibliography: set grid.cell(breakable: false)
#bibliography("refs.bib", style: "ieee")
