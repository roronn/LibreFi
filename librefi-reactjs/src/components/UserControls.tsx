import { memo, type ReactElement } from "react";
import Coin from "#components/Coin";
import More from "#components/More";
import Rates from "#components/Rates";
import Voucher from "#components/Voucher";
import { Button } from "#components/ui/button";
import { CoinsIcon, EllipsisVertical, PauseIcon, PlayIcon, Ticket, PhilippinePeso } from "lucide-react";
import type { UserType } from "../types/user";

type UserControlsProps = {
    user: UserType | undefined;
    onRefresh: () => void;
    onPlayPause: () => void;
    onUpdateUser: (expires_on: number, now: number) => void;
};

function UserControls({ user, onRefresh, onPlayPause, onUpdateUser }: UserControlsProps) {
    return (
        <div className="w-full flex flex-col items-center gap-5 pt-12">
            <div className="w-full flex flex-col items-center">
                <Coin onClose={onRefresh}>
                    <Button
                        variant="default"
                        className="w-full h-14 rounded-xl bg-[#83E0B5] text-[#172C25] font-bold text-base hover:bg-[#70D6A5] flex items-center justify-center gap-2 shadow-none"
                    >
                        <CoinsIcon size={22} />
                        INSERT COIN
                    </Button>
                </Coin>
            </div>
            <div className="flex items-center justify-center gap-5">
                <div className="flex flex-col items-center">
                    <Button onClick={onPlayPause} variant="outline" size="icon-lg" className="hover:bg-primary/5 hover:text-primary/90">
                        {user?.paused ? <PlayIcon className="text-green-500" /> : <PauseIcon className="text-yellow-400" />}
                    </Button>
                    <span className="text-[0.7rem] text-center mt-1 text-foreground/60">{user?.paused ? "PLAY" : "PAUSE"}</span>
                </div>
                <div className="flex flex-col items-center">
                    <Voucher updateUser={onUpdateUser}>
                        <Button variant="outline" size="icon-lg" className="hover:bg-primary/5 hover:text-primary/90">
                            <Ticket />
                        </Button>
                    </Voucher>
                    <span className="text-[0.7rem] text-center mt-1 text-foreground/60">VOUCHER</span>
                </div>
                <div className="flex flex-col items-center">
                    <Rates>
                        <Button variant="outline" size="icon-lg" className="hover:bg-primary/5 hover:text-primary/90">
                            <PhilippinePeso />
                        </Button>
                    </Rates>
                    <span className="text-[0.7rem] text-center mt-1 text-foreground/60">RATES</span>
                </div>
                <div className="flex flex-col items-center">
                    <More user={user}>
                        <Button variant="outline" size="icon-lg" className="hover:bg-primary/5 hover:text-primary/90">
                            <EllipsisVertical />
                        </Button>
                    </More>
                    <span className="text-[0.7rem] text-center mt-1 text-foreground/60">OTHER</span>
                </div>
            </div>
        </div>
    );
}

export default memo(UserControls);
