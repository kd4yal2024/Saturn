`default_nettype none

module watchdog_formal (
    input wire aclk
);
    localparam integer TIMEOUT = 4;

    (* anyseq *) reg aresetn;
    (* anyseq *) reg activity1;
    (* anyseq *) reg activity2;
    wire TXEnable;
    wire [31:0] formal_counter;

    Watchdog #(
        .TimeoutClocks(TIMEOUT)
    ) dut (
        .aclk(aclk),
        .aresetn(aresetn),
        .activity1(activity1),
        .activity2(activity2),
        .TXEnable(TXEnable),
        .formal_counter(formal_counter)
    );

    reg f_past_valid = 1'b0;
    reg [7:0] inactive_cycles = 8'd0;
    reg saw_activity = 1'b0;

    always @(posedge aclk) begin
        f_past_valid <= 1'b1;

        // Begin in reset, release it on the next formal step, and keep it released.
        if (!f_past_valid)
            assume(!aresetn);
        else
            assume(aresetn);

        if (!aresetn || activity1 || activity2)
            inactive_cycles <= 8'd0;
        else if (inactive_cycles != 8'hff)
            inactive_cycles <= inactive_cycles + 1'b1;

        if (activity1 || activity2)
            saw_activity <= 1'b1;

        if (f_past_valid) begin
            if (!$past(aresetn)) begin
                assert(!TXEnable);
                assert(formal_counter == 0);
            end

            if ($past(aresetn && (activity1 || activity2))) begin
                assert(TXEnable);
                assert(formal_counter == TIMEOUT);
            end

            if ($past(aresetn && !(activity1 || activity2) && formal_counter != 0)) begin
                assert(TXEnable);
                assert(formal_counter == $past(formal_counter) - 1'b1);
            end

            if ($past(aresetn && !(activity1 || activity2) && formal_counter == 0)) begin
                assert(!TXEnable);
                assert(formal_counter == 0);
            end
        end

        if (f_past_valid && aresetn) begin
            assert(formal_counter <= TIMEOUT);

            // TX may remain enabled for TIMEOUT quiet clocks, never longer.
            if (inactive_cycles > TIMEOUT)
                assert(!TXEnable);

            if (TXEnable)
                assert(inactive_cycles <= TIMEOUT);

            if (!TXEnable)
                assert(formal_counter == 0);
        end

        cover(saw_activity && !TXEnable && inactive_cycles > TIMEOUT);
    end
endmodule

`default_nettype wire
