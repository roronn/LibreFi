import { FingerprintIcon, Router, Shuffle, SunMoonIcon, ArrowRight, RefreshCcw } from "lucide-react";
import {
    Popover,
    PopoverContent,
    PopoverTrigger,
} from "../components/ui/popover";
import type { ReactElement } from "react";
import type { UserType } from "../types/user";

export function toggleTheme() {
    const theme = window.localStorage.getItem("theme") || "light";
    const nextTheme = theme === "light" ? "dark" : "light";
    document.documentElement.classList = nextTheme as string;
    window.localStorage.setItem("theme", nextTheme);
}

function More({ user, children }: { user: UserType | undefined, children: ReactElement }) {
    return (
        <Popover>
            <PopoverTrigger render={children}>Open Popover</PopoverTrigger>
            <PopoverContent className="p-0 w-48">
                <div>
                    <div className="p-2 font-semibold text-xs">Details</div>
                    <div className="p-3 flex gap-4 border-b border-t border-foreground/5">
                        <Router size="17" />
                        <span>{user?.ip}</span>
                    </div>
                    <div className="p-3 flex gap-4 border-b border-foreground/5">
                        <FingerprintIcon size="17" />
                        <span>{user?.mac}</span>
                    </div>
                    <div className="p-2 font-semibold text-xs">Actions</div>
                    <button onClick={toggleTheme} className="w-full text-left p-3 flex gap-4 border-b border-t border-foreground/5 cursor-pointer hover:bg-primary/5 transition-colors duration-75">
                        <SunMoonIcon size="17" />
                        <span className="grow">Toggle Theme</span>
                        <Shuffle size="17" className="mt-px" />
                    </button>
                    <div className="p-3 flex gap-4 border-b border-foreground/5 cursor-pointer hover:bg-primary/5 transition-colors duration-75">
                        <RefreshCcw size="17" />
                        <span className="grow">Fix Connection</span>
                        <ArrowRight size="17" className="mt-px" />
                    </div>
                </div>
            </PopoverContent>
        </Popover>
    );
}

export default More;
