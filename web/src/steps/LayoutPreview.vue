<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { Layout, LayoutAnchor } from "../types";

const props = defineProps<{
  layout: Layout;
  slideUrl: string | null;
  personUrl: string | null;
  personKind: "alpha" | "card" | "none";
  canvasWidth: number;
  canvasHeight: number;
  editable?: boolean;
}>();

const emit = defineEmits<{
  (e: "ready", value: LayoutAnchor): void;
  (e: "update", value: LayoutAnchor): void;
  (e: "commit", value: LayoutAnchor): void;
}>();

const canvasEl = ref<HTMLElement | null>(null);
const personAspect = ref(0.75); // width / height until the real frame reports
const rightBar = computed(
  () => props.layout.mode !== "free" && props.layout.position === "right_bar"
);
const circular = computed(
  () =>
    !rightBar.value &&
    props.personKind !== "card" &&
    props.layout.shape === "circle"
);
const cardPad = 16;

const maxContentHeight = computed(
  () =>
    props.canvasHeight -
    (props.personKind === "card" ? cardPad * 2 : 0)
);
const requestedWidth = computed(
  () => (props.canvasWidth * props.layout.width_pct) / 100
);
// Free placement is capped by the canvas height in render; match that here.
const contentWidth = computed(() =>
  props.layout.mode === "free"
    ? Math.min(requestedWidth.value, maxContentHeight.value * Math.max(personAspect.value, 0.1))
    : requestedWidth.value
);
const rawContentHeight = computed(
  () => contentWidth.value / Math.max(personAspect.value, 0.1)
);
// Render's circle branch center-crops the scaled person to a square before masking.
const visualWidth = computed(() =>
  circular.value
    ? Math.min(contentWidth.value, rawContentHeight.value)
    : contentWidth.value
);
const visualHeight = computed(() =>
  circular.value
    ? Math.min(contentWidth.value, rawContentHeight.value)
    : rawContentHeight.value
);
const outerWidth = computed(() =>
  rightBar.value
    ? contentWidth.value
    : visualWidth.value + (props.personKind === "card" ? cardPad * 2 : 0)
);
const outerHeight = computed(() =>
  rightBar.value
    ? props.canvasHeight
    : visualHeight.value + (props.personKind === "card" ? cardPad * 2 : 0)
);

/** Pixel rectangle in project units; render uses these same normalized rules. */
const geometry = computed(() => {
  if (rightBar.value) {
    return {
      left: props.canvasWidth - contentWidth.value,
      top: 0,
      width: contentWidth.value,
      height: props.canvasHeight,
    };
  }
  let left: number;
  let top: number;
  if (props.layout.mode === "free") {
    left =
      ((props.canvasWidth - outerWidth.value) * (props.layout.x_pct ?? 0)) / 100;
    top =
      ((props.canvasHeight - outerHeight.value) * (props.layout.y_pct ?? 0)) /
      100;
  } else {
    // Legacy presets use the original 80 px / 60 px output margins.
    left =
      props.layout.position === "bottom_left"
        ? 80
        : props.canvasWidth - outerWidth.value - 80;
    top = props.canvasHeight - outerHeight.value - 60;
  }
  return {
    left: Math.max(0, left),
    top: Math.max(0, top),
    width: outerWidth.value,
    height: outerHeight.value,
  };
});

const style = computed(() => {
  const g = geometry.value;
  let borderRadius = "0";
  if (!rightBar.value) {
    if (props.personKind !== "card" && props.layout.shape === "circle") {
      borderRadius = "50%";
    } else {
      // same circular radius as the renderer: min(W,H)/8; the card branch always
      // uses the rounded-card mask regardless of the circle option.
      const radius = Math.min(g.width, g.height) / 8;
      borderRadius = `${(radius / g.width) * 100}% / ${(radius / g.height) * 100}%`;
    }
  }
  return {
    left: `${(g.left / props.canvasWidth) * 100}%`,
    top: `${(g.top / props.canvasHeight) * 100}%`,
    width: `${(g.width / props.canvasWidth) * 100}%`,
    height: `${(g.height / props.canvasHeight) * 100}%`,
    borderRadius,
  };
});

const mediaStyle = computed(() => {
  if (props.personKind !== "card") return {};
  const g = geometry.value;
  return {
    left: `${(cardPad / g.width) * 100}%`,
    top: `${(cardPad / g.height) * 100}%`,
    width: `${(visualWidth.value / g.width) * 100}%`,
    height: `${(visualHeight.value / g.height) * 100}%`,
  };
});

const personClass = computed(() => ({
  "is-card": props.personKind === "card",
  "is-right-bar": rightBar.value,
}));

/** Free-space anchor equivalent of the current placement, for the sliders. */
const anchor = computed(() => {
  if (props.layout.mode === "free") {
    return {
      x_pct: props.layout.x_pct ?? 0,
      y_pct: props.layout.y_pct ?? 0,
      width_pct: props.layout.width_pct,
    };
  }
  if (rightBar.value) {
    return { x_pct: 100, y_pct: 0, width_pct: props.layout.width_pct };
  }
  const g = geometry.value;
  const xSpace = props.canvasWidth - g.width;
  const ySpace = props.canvasHeight - g.height;
  return {
    x_pct: xSpace > 0 ? Math.min(100, Math.max(0, (g.left / xSpace) * 100)) : 0,
    y_pct: ySpace > 0 ? Math.min(100, Math.max(0, (g.top / ySpace) * 100)) : 0,
    width_pct: props.layout.width_pct,
  };
});

watch(
  () => [props.layout, props.personKind, personAspect.value] as const,
  () => emit("ready", anchor.value),
  { immediate: true, deep: true }
);

function onPersonMedia(event: Event) {
  const el = event.target as HTMLImageElement | HTMLVideoElement;
  const width =
    "naturalWidth" in el ? el.naturalWidth : el.videoWidth;
  const height =
    "naturalHeight" in el ? el.naturalHeight : el.videoHeight;
  if (width > 0 && height > 0) personAspect.value = width / height;
}

type DragState = {
  mode: "move" | "resize";
  pointerId: number;
  startClientX: number;
  startClientY: number;
  startLeft: number;
  startTop: number;
  startWidthPct: number;
  offsetX: number;
  offsetY: number;
  scaleX: number;
  scaleY: number;
  moved: boolean;
};
const drag = ref<DragState | null>(null);
const lastAnchor = ref<LayoutAnchor | null>(null);
const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

function metrics(event: PointerEvent) {
  const rect = canvasEl.value!.getBoundingClientRect();
  return {
    rect,
    scaleX: props.canvasWidth / rect.width,
    scaleY: props.canvasHeight / rect.height,
  };
}

function startMove(event: PointerEvent) {
  if (!props.editable || !canvasEl.value) return;
  event.preventDefault();
  const { rect, scaleX, scaleY } = metrics(event);
  const g = geometry.value;
  drag.value = {
    mode: "move",
    pointerId: event.pointerId,
    startClientX: event.clientX,
    startClientY: event.clientY,
    startLeft: g.left,
    startTop: g.top,
    startWidthPct: props.layout.width_pct,
    offsetX: (event.clientX - rect.left) * scaleX - g.left,
    offsetY: (event.clientY - rect.top) * scaleY - g.top,
    scaleX,
    scaleY,
    moved: false,
  };
  try {
    canvasEl.value.setPointerCapture(event.pointerId);
  } catch {
    // synthetic events have no active pointer; real drags capture normally
  }
}

function startResize(event: PointerEvent) {
  if (!props.editable || !canvasEl.value) return;
  event.preventDefault();
  event.stopPropagation();
  const { scaleX, scaleY } = metrics(event);
  drag.value = {
    mode: "resize",
    pointerId: event.pointerId,
    startClientX: event.clientX,
    startClientY: event.clientY,
    startLeft: geometry.value.left,
    startTop: geometry.value.top,
    startWidthPct: props.layout.width_pct,
    offsetX: 0,
    offsetY: 0,
    scaleX,
    scaleY,
    moved: false,
  };
  try {
    canvasEl.value.setPointerCapture(event.pointerId);
  } catch {
    // synthetic events have no active pointer; real drags capture normally
  }
}

function onPointerMove(event: PointerEvent) {
  const d = drag.value;
  if (!d || d.pointerId !== event.pointerId || !canvasEl.value) return;
  event.preventDefault();
  if (d.mode === "move") {
    const left = clamp(
      (event.clientX - d.startClientX) * d.scaleX + d.startLeft,
      0,
      props.canvasWidth - outerWidth.value
    );
    const top = clamp(
      (event.clientY - d.startClientY) * d.scaleY + d.startTop,
      0,
      props.canvasHeight - outerHeight.value
    );
    const xSpace = props.canvasWidth - outerWidth.value;
    const ySpace = props.canvasHeight - outerHeight.value;
    lastAnchor.value = {
      x_pct: xSpace > 0 ? (left / xSpace) * 100 : 0,
      y_pct: ySpace > 0 ? (top / ySpace) * 100 : 0,
      width_pct: props.layout.width_pct,
    };
  } else {
    const delta = (event.clientX - d.startClientX) * d.scaleX;
    const widthPct = clamp(
      d.startWidthPct + (delta / props.canvasWidth) * 100,
      5,
      40
    );
    lastAnchor.value = {
      x_pct: anchor.value.x_pct,
      y_pct: anchor.value.y_pct,
      width_pct: widthPct,
    };
  }
  d.moved = true;
  if (lastAnchor.value) emit("update", lastAnchor.value);
}

function onPointerUp(event: PointerEvent) {
  const d = drag.value;
  if (!d || d.pointerId !== event.pointerId) return;
  drag.value = null;
  if (d.moved && lastAnchor.value) emit("commit", lastAnchor.value);
  lastAnchor.value = null;
}
</script>

<template>
  <div
    ref="canvasEl"
    class="layout-canvas"
    :class="{ editable }"
    :style="{ aspectRatio: `${canvasWidth} / ${canvasHeight}` }"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp"
    @pointercancel="onPointerUp"
  >
    <img v-if="slideUrl" class="slide" :src="slideUrl" alt="课件首页预览" />
    <div v-else class="slide slide-empty">课件首页</div>

    <div
      class="person"
      :class="[personClass, { draggable: editable }]"
      :style="style"
      @pointerdown="startMove"
    >
      <img
        v-if="personKind === 'alpha' && personUrl"
        class="person-media"
        :style="mediaStyle"
        :src="personUrl"
        alt="人像位置预览"
        draggable="false"
        @load="onPersonMedia"
      />
      <video
        v-else-if="personKind === 'card' && personUrl"
        class="person-media"
        :style="mediaStyle"
        :src="personUrl"
        autoplay
        muted
        loop
        playsinline
        @loadedmetadata="onPersonMedia"
      />
      <div v-else class="person-placeholder">人像</div>
      <span
        v-if="editable"
        class="resize-handle"
        title="拖动缩放"
        @pointerdown="startResize"
      ></span>
    </div>
  </div>
</template>

<style scoped>
.layout-canvas {
  position: relative;
  width: 100%;
  overflow: hidden;
  background: #fff;
  border: 1px solid #dcdfe6;
  border-radius: 8px;
  user-select: none;
}
.layout-canvas.editable {
  touch-action: none;
}
.slide {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: contain;
  background: #fff;
}
.slide-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  color: #909399;
  font-size: 13px;
}
.person {
  position: absolute;
  overflow: hidden;
  box-sizing: border-box;
  background: transparent;
}
.person.is-card {
  background: #fff;
}
.person.is-right-bar {
  border-radius: 0;
}
.person-media {
  position: absolute;
  left: 0;
  top: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.person-placeholder {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  background: #3a7bd5;
  font-size: 12px;
}
.person.draggable {
  cursor: move;
  touch-action: none;
  outline: 1px dashed rgba(64, 158, 255, 0.55);
  outline-offset: 2px;
}
.resize-handle {
  position: absolute;
  right: -7px;
  bottom: -7px;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: #409eff;
  border: 2px solid #fff;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.35);
  cursor: nwse-resize;
  touch-action: none;
}
</style>
