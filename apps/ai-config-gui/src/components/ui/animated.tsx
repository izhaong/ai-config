import { AnimatePresence, motion } from "motion/react";
import type { ReactNode } from "react";

import { useMotionPresets } from "@/hooks/useMotionPresets";
import { cn } from "@/lib/utils";

interface AnimatedOverlayProps {
  open: boolean;
  className?: string;
  onClick?: () => void;
  children: ReactNode;
}

/** 遮罩层：内置 AnimatePresence，支持 exit 动画 */
export function AnimatedOverlay({
  open,
  className,
  onClick,
  children,
}: AnimatedOverlayProps) {
  const { backdrop } = useMotionPresets();

  return (
    <AnimatePresence>
      {open ? (
        <motion.div
          key="overlay"
          className={className}
          onClick={onClick}
          initial={backdrop.initial}
          animate={backdrop.animate}
          exit={backdrop.exit}
          transition={backdrop.transition}
        >
          {children}
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}

interface AnimatedScaleDialogProps {
  className?: string;
  role?: string;
  "aria-modal"?: boolean | "true" | "false";
  children: ReactNode;
}

/** 居中弹窗：淡入 + 轻微缩放 */
export function AnimatedScaleDialog({
  className,
  role = "dialog",
  "aria-modal": ariaModal = true,
  children,
}: AnimatedScaleDialogProps) {
  const { scalePanel } = useMotionPresets();

  return (
    <motion.div
      className={className}
      role={role}
      aria-modal={ariaModal}
      onClick={(e) => e.stopPropagation()}
      initial={scalePanel.initial}
      animate={scalePanel.animate}
      exit={scalePanel.exit}
      transition={scalePanel.transition}
    >
      {children}
    </motion.div>
  );
}

interface AnimatedSlidePanelProps {
  className?: string;
  role?: string;
  "aria-modal"?: boolean | "true" | "false";
  "aria-labelledby"?: string;
  children: ReactNode;
}

/** 侧滑面板（抽屉） */
export function AnimatedSlidePanel({
  className,
  role = "dialog",
  "aria-modal": ariaModal = true,
  "aria-labelledby": ariaLabelledby,
  children,
}: AnimatedSlidePanelProps) {
  const { slidePanel } = useMotionPresets();

  return (
    <motion.aside
      className={className}
      role={role}
      aria-modal={ariaModal}
      aria-labelledby={ariaLabelledby}
      onClick={(e) => e.stopPropagation()}
      initial={slidePanel.initial}
      animate={slidePanel.animate}
      exit={slidePanel.exit}
      transition={slidePanel.transition}
    >
      {children}
    </motion.aside>
  );
}

interface AnimatedListProps {
  className?: string;
  listKey?: string;
  children: ReactNode;
}

/** 列表容器：子项 stagger 入场 */
export function AnimatedList({ className, listKey, children }: AnimatedListProps) {
  const { listContainer } = useMotionPresets();

  return (
    <motion.div
      key={listKey}
      className={cn(className)}
      variants={listContainer}
      initial="hidden"
      animate="visible"
    >
      {children}
    </motion.div>
  );
}

interface AnimatedListItemProps {
  className?: string;
  layout?: boolean;
  children: ReactNode;
}

export function AnimatedListItem({
  className,
  layout = true,
  children,
}: AnimatedListItemProps) {
  const { listItem, layoutTransition } = useMotionPresets();

  return (
    <motion.div
      className={className}
      variants={listItem}
      layout={layout}
      transition={layoutTransition}
    >
      {children}
    </motion.div>
  );
}

export { AnimatePresence };
