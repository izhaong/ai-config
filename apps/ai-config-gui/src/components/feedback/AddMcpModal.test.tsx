import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AddMcpModal } from "./AddMcpModal";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const VALID_JSON = `{
  "ai-config": {
    "command": "ai-config",
    "args": ["serve"]
  }
}`;

describe("AddMcpModal", () => {
  afterEach(() => {
    cleanup();
  });

  it("renders json textarea and disabled submit when empty", () => {
    render(<AddMcpModal onCancel={vi.fn()} onSubmit={vi.fn()} />);
    expect(screen.getByLabelText("addMcp.jsonLabel")).toBeTruthy();
    expect(screen.getByRole("button", { name: "addMcp.submit" })).toHaveProperty(
      "disabled",
      true,
    );
  });

  it("submits parsed server config", () => {
    const onSubmit = vi.fn();
    render(<AddMcpModal onCancel={vi.fn()} onSubmit={onSubmit} />);

    fireEvent.change(screen.getByLabelText("addMcp.jsonLabel"), {
      target: { value: VALID_JSON },
    });
    fireEvent.click(screen.getByRole("button", { name: "addMcp.submit" }));

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit).toHaveBeenCalledWith(
      "ai-config",
      expect.stringContaining('"command": "ai-config"'),
    );
  });

  it("shows validation error for invalid json", () => {
    render(<AddMcpModal onCancel={vi.fn()} onSubmit={vi.fn()} />);

    fireEvent.change(screen.getByLabelText("addMcp.jsonLabel"), {
      target: { value: "{ not-json" },
    });
    fireEvent.click(screen.getByRole("button", { name: "addMcp.submit" }));

    expect(screen.getByText(/JSON/)).toBeTruthy();
  });

  it("calls onCancel when backdrop is clicked", () => {
    const onCancel = vi.fn();
    const { container } = render(
      <AddMcpModal onCancel={onCancel} onSubmit={vi.fn()} />,
    );
    fireEvent.click(container.querySelector(".confirm-backdrop")!);
    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});
