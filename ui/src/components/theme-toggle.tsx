
import { Moon, Sun } from "lucide-react";
import { useTheme } from "@/hooks/use-theme";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";

interface ThemeToggleProps {
  asChild?: boolean;
  className?: string;
}

export function ThemeToggle({ asChild, className }: ThemeToggleProps) {
  const { theme, setTheme } = useTheme();
  const Comp = asChild ? "div" : Button;

  const toggleTheme = () => setTheme(theme === "light" ? "dark" : "light");

  const content = (
    <div className="flex items-center gap-2">
      <div className="relative w-4 h-4">
        <Sun className="absolute h-4 w-4 rotate-0 scale-100 transition-all dark:-rotate-90 dark:scale-0" />
        <Moon className="absolute h-4 w-4 rotate-90 scale-0 transition-all dark:rotate-0 dark:scale-100" />
      </div>
      <span>Toggle Theme</span>
      <span className="sr-only">Toggle theme</span>
    </div>
  );

  if (asChild) {
    return (
      <div
        className={`${className} flex items-center`}
        onClick={toggleTheme}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            toggleTheme();
          }
        }}
      >
        {content}
      </div>
    );
  }

  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <Comp
            variant="ghost"
            onClick={toggleTheme}
            className={`${className} flex items-center justify-start w-full px-2`}
          >
            {content}
          </Comp>
        </TooltipTrigger>
        <TooltipContent>
          <p>Toggle theme</p>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
