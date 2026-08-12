# Indicator Preferences Compact Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remover os grandes vazios e desalinhamentos das preferências do indicador, fazendo os editores opcionais ocuparem espaço somente quando visíveis.

**Architecture:** Um modelo puro em `preferences_view/layout.rs` calcula slots verticais a partir da visibilidade dos editores de CPU, RAM e rótulos. A camada AppKit aplica esses slots aos controles existentes e redimensiona o documento rolável; nenhum contrato de preferência, evento ou runtime muda.

**Tech Stack:** Rust 2021, AppKit via `objc2-app-kit`, testes unitários com `cargo test`, empacotamento macOS existente.

## Global Constraints

- CPU e RAM formam um grupo visual **Cores**; o reset combinado fica no cabeçalho desse grupo.
- Rótulos e seletores de CPU/RAM compartilham coluna e baseline.
- Editores de cor ocultos não reservam altura.
- Espaçamento interno deve permanecer entre 12 e 16 pt; grupos usam 24 pt.
- Prévias, rodapé, Disco e Mole, preferências, renderer e coleta de métricas não mudam.
- O ADR 0001 continua valendo: nenhum timer, polling, worker ou trabalho recorrente novo.
- Todos os comandos de shell devem começar literalmente com `rtk`.

---

## File Structure

- Create `src/preferences_view/layout.rs`: geometria pura e constantes do fluxo vertical.
- Modify `src/preferences_view.rs`: reexportar somente os tipos de layout consumidos pela UI AppKit.
- Modify `src/macos/windows/preferences/indicator.rs`: organizar controles no grupo **Cores** e aplicar a geometria calculada.
- Modify `src/macos/windows/preferences/mod.rs`: sincronizar altura do controle, stack e document view rolável.

### Task 1: Modelo puro de geometria compacta

**Files:**
- Create: `src/preferences_view/layout.rs`
- Modify: `src/preferences_view.rs:1-5`
- Test: `src/preferences_view/layout.rs` (`#[cfg(test)]` no mesmo módulo)

**Interfaces:**
- Consumes: três booleanos de visibilidade provenientes de `IndicatorPreferences`.
- Produces: `IndicatorControlsVisibility`, `VerticalSlot`, `RowSlot`, `ControlSlot` e `IndicatorControlsLayout::new(visibility)`.

- [ ] **Step 1: Escrever testes RED para compactação, deslocamento e alinhamento**

Criar os testes antes da implementação:

```rust
#[test]
fn dynamic_colors_produce_the_smallest_layout_without_editor_slots() {
    let layout = IndicatorControlsLayout::new(IndicatorControlsVisibility::default());

    assert_eq!(layout.cpu_editor(), None);
    assert_eq!(layout.ram_editor(), None);
    assert_eq!(layout.labels_editor(), None);
    assert!(layout.cpu_row().bottom() <= layout.ram_row().top());
    assert!(layout.ram_row().bottom() <= layout.labels_heading().top());
}

#[test]
fn each_visible_editor_only_pushes_the_content_after_its_metric() {
    let compact = IndicatorControlsLayout::new(IndicatorControlsVisibility::default());
    let cpu_fixed = IndicatorControlsLayout::new(IndicatorControlsVisibility {
        cpu_editor: true,
        ..IndicatorControlsVisibility::default()
    });

    assert_eq!(cpu_fixed.cpu_row(), compact.cpu_row());
    assert_eq!(cpu_fixed.cpu_editor().unwrap().height(), COLOR_EDITOR_HEIGHT);
    assert_eq!(
        cpu_fixed.ram_row().top() - compact.ram_row().top(),
        COLOR_EDITOR_HEIGHT + INLINE_GAP
    );
}

#[test]
fn cpu_and_ram_rows_share_columns_and_the_reset_belongs_to_colors() {
    let layout = IndicatorControlsLayout::new(IndicatorControlsVisibility::default());

    assert_eq!(layout.cpu_row().label_x(), layout.ram_row().label_x());
    assert_eq!(layout.cpu_row().control_x(), layout.ram_row().control_x());
    assert_eq!(layout.colors_reset().vertical(), layout.colors_heading());
    assert!(layout.colors_reset().x() > layout.cpu_row().control_x());
    assert_eq!(GROUP_GAP, 24.0);
}
```

- [ ] **Step 2: Executar os testes e confirmar RED pela API ausente**

Run: `rtk cargo test --lib preferences_view::layout::tests --locked`

Expected: FAIL porque `preferences_view::layout`, `IndicatorControlsLayout` e seus getters ainda não existem.

- [ ] **Step 3: Implementar o cursor top-down e os slots explícitos**

Em `src/preferences_view.rs`, declarar e reexportar o submódulo:

```rust
mod layout;

pub use layout::{
    ControlSlot, IndicatorControlsLayout, IndicatorControlsVisibility, RowSlot, VerticalSlot,
};
```

Em `src/preferences_view/layout.rs`, implementar os tipos baseados em offsets a partir do topo:

```rust
pub const INLINE_GAP: f64 = 12.0;
pub const GROUP_GAP: f64 = 24.0;
pub const COLOR_EDITOR_HEIGHT: f64 = 160.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IndicatorControlsVisibility {
    pub cpu_editor: bool,
    pub ram_editor: bool,
    pub labels_editor: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VerticalSlot {
    top: f64,
    height: f64,
}

impl VerticalSlot {
    pub const fn top(self) -> f64 { self.top }
    pub const fn height(self) -> f64 { self.height }
    pub const fn bottom(self) -> f64 { self.top + self.height }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowSlot {
    vertical: VerticalSlot,
    label_x: f64,
    control_x: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlSlot {
    vertical: VerticalSlot,
    x: f64,
    width: f64,
}
```

`IndicatorControlsLayout::new` deve usar um cursor top-down, inserir `INLINE_GAP + COLOR_EDITOR_HEIGHT` somente para cada editor visível e inserir `GROUP_GAP` uma vez entre **Cores**, **Rótulos**, **Tipografia** e **Atualização**. Expor getters para os seguintes slots:

```text
colors_heading, colors_reset, cpu_row, cpu_editor, ram_row, ram_editor,
labels_heading, labels_visibility_row, labels_mode_row, labels_editor,
typography_heading, family_row, size_row, weight_row,
font_fallback_warning, layout_warning,
update_heading, interval_row, interval_help, interval_error
```

As linhas CPU/RAM usam `label_x = 0.0`, `control_x = 100.0`, altura `28.0`; cabeçalhos usam `24.0`; `colors_reset` usa `x = 390.0`, `width = 160.0`; `content_height` termina após o slot de erro de intervalo, sem padding fantasma.

- [ ] **Step 4: Executar testes focados e confirmar GREEN**

Run: `rtk cargo test --lib preferences_view::layout::tests --locked`

Expected: todos os testes do módulo passam.

- [ ] **Step 5: Executar testes existentes do view model**

Run: `rtk cargo test --test preferences_view --locked`

Expected: os testes existentes continuam passando sem mudança de comportamento.

- [ ] **Step 6: Commitar o modelo puro**

```bash
rtk git add src/preferences_view.rs src/preferences_view/layout.rs
rtk git commit -m "feat: model compact indicator preferences layout"
```

### Task 2: Aplicar o fluxo compacto na janela AppKit

**Files:**
- Modify: `src/macos/windows/preferences/indicator.rs:270-750`
- Modify: `src/macos/windows/preferences/mod.rs:230-236, 359-438, 445-500`
- Test: `src/preferences_view/layout.rs`

**Interfaces:**
- Consumes: `IndicatorControlsLayout::new(IndicatorControlsVisibility)` da Task 1.
- Produces: `IndicatorControls::content_height()`, `IndicatorControls::apply_layout(...)` e `IndicatorPage::apply_preferences(...)`, mantendo os eventos e a ordem de teclado existentes.

- [ ] **Step 1: Escrever um teste RED para a conversão de coordenadas AppKit**

Adicionar primeiro ao módulo puro:

```rust
#[test]
fn top_down_slots_translate_to_appkit_without_changing_row_alignment() {
    let layout = IndicatorControlsLayout::new(IndicatorControlsVisibility::default());
    let cpu = layout.cpu_row();
    let ram = layout.ram_row();

    assert_eq!(
        cpu.origin_y(layout.content_height()),
        layout.content_height() - cpu.bottom()
    );
    assert_eq!(cpu.label_origin_y(layout.content_height()), cpu.control_origin_y(layout.content_height()));
    assert_eq!(ram.label_origin_y(layout.content_height()), ram.control_origin_y(layout.content_height()));
}
```

- [ ] **Step 2: Executar o teste e confirmar RED pelos adapters ausentes**

Run: `rtk cargo test --lib preferences_view::layout::tests::top_down_slots_translate_to_appkit_without_changing_row_alignment --locked`

Expected: FAIL porque `origin_y`, `label_origin_y` e `control_origin_y` ainda não existem.

- [ ] **Step 3: Reorganizar os controles CPU/RAM sem alterar suas actions**

Em `IndicatorControls::new`:

```rust
let colors_heading = heading(mtm, "Cores", 0.0);
let cpu_label = text_label(mtm, "CPU", 0.0);
let ram_label = text_label(mtm, "RAM", 0.0);
```

Remover os cabeçalhos separados `CPU`/`RAM`, colocar `reset_cpu_and_ram` na linha de `colors_heading` e manter `cpu_mode`/`ram_mode` na mesma coluna de controle. Reter labels, headings e textos auxiliares em uma struct privada `IndicatorLayoutViews`, para que o relayout não procure subviews por ordem ou texto.

- [ ] **Step 4: Aplicar slots e remover as coordenadas verticais fixas**

Adicionar ao `IndicatorControls`:

```rust
fn apply_layout(&self, visibility: IndicatorControlsVisibility) {
    let layout = IndicatorControlsLayout::new(visibility);
    self.layout_views.apply(&layout);
    self.view.setFrameSize(NSSize::new(560.0, layout.content_height()));
    self.content_height.set(layout.content_height());
}

pub(super) fn content_height(&self) -> f64 {
    self.content_height.get()
}
```

Implementar os adapters testados em `VerticalSlot`/`RowSlot`. `IndicatorLayoutViews::apply` usa esses adapters para converter offsets top-down para AppKit:

```rust
fn origin_y(content_height: f64, slot: VerticalSlot) -> f64 {
    content_height - slot.bottom()
}
```

Depois de atualizar `setHidden`, `IndicatorControls::apply` chama:

```rust
self.apply_layout(IndicatorControlsVisibility {
    cpu_editor: cpu_fixed,
    ram_editor: ram_fixed,
    labels_editor: labels_fixed,
});
```

Não mudar selectors, accessibility identifiers, envio de eventos nem a ordem de `setNextKeyView`.

- [ ] **Step 5: Redimensionar toda a cadeia rolável**

Reter `groups_document` e `groups_stack` em `IndicatorPage` e centralizar o resize:

```rust
impl IndicatorPage {
    fn apply_preferences(&self, preferences: &IndicatorPreferences) {
        self.controls.apply(preferences);
        let controls_height = self.controls.content_height();
        self.controls
            .view()
            .setFrameSize(NSSize::new(560.0, controls_height));
        self.groups_stack.setFrame(NSRect::new(
            NSPoint::new(16.0, 16.0),
            NSSize::new(580.0, controls_height),
        ));
        self.groups_document
            .setFrameSize(NSSize::new(612.0, controls_height + 32.0));
    }
}
```

Trocar `self.indicator.controls.apply(...)` por `self.indicator.apply_preferences(...)`. Na criação, derivar as alturas iniciais de `controls.content_height()` em vez de usar `1200.0`, `1168.0` e `1160.0` como canvas permanente.

- [ ] **Step 6: Confirmar GREEN e compilação AppKit**

```bash
rtk cargo test --lib preferences_view::layout::tests --locked
rtk cargo test --bin statlet --locked
rtk cargo test --test preferences_view --test indicator_preferences_flow --locked
```

Expected: todos passam; nenhuma warning nova.

- [ ] **Step 7: Executar gates completos**

```bash
rtk cargo fmt --all -- --check
rtk cargo test --all-targets --all-features --locked
rtk cargo clippy --all-targets --all-features --locked -- -D warnings
rtk git diff --check
```

Expected: todos passam sem warnings ou whitespace errors.

- [ ] **Step 8: Empacotar, reiniciar e validar visualmente**

Run: `rtk bash scripts/package-release.sh dist`

Validar o caminho e o PID exatos antes de encerrar o processo atualmente executado. Encerrar somente o executável de `dist/Statlet.app`, abrir o bundle recém-gerado e confirmar:

- modo dinâmico sem espaços reservados;
- CPU/RAM alinhadas na mesma grade;
- reset no cabeçalho **Cores**;
- cada modo **Fixa** revela o editor logo abaixo e desloca apenas o conteúdo posterior;
- scroll alcança todos os grupos sem área vazia artificial;
- alternar de volta a **Dinâmica** recolhe o editor.

Se a automação não expuser a menu bar ou a janela acessível, manter a checagem visual como gate manual e relatar explicitamente essa limitação.

- [ ] **Step 9: Commitar a correção AppKit**

```bash
rtk git add src/macos/windows/preferences/indicator.rs src/macos/windows/preferences/mod.rs
rtk git commit -m "fix: compact indicator preferences layout"
```

## Final Review Checklist

- [ ] O estado totalmente dinâmico não reserva nenhum editor de 160 pt.
- [ ] CPU e RAM usam as mesmas colunas e baseline.
- [ ] O reset combinado pertence visualmente ao grupo **Cores**.
- [ ] Cada editor fixo desloca somente o conteúdo posterior.
- [ ] A document view acompanha a altura real dos controles.
- [ ] Tab/Shift-Tab continuam ignorando editores ocultos.
- [ ] Preferências, renderer, preview, Disco/Mole e runtime não mudaram.
- [ ] Suíte, Clippy, formatação, bundle e processo reiniciado têm evidência atual.
